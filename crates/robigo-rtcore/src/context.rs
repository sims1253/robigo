//! Contexts and unwinding (docs/g3.md §6.5): GNU R's chain of `RCNTXT`
//! records, the pending-exit stack's parking, and the walk every exit makes
//! (`R_run_onexits` before each jump, `R462/main/context.c`).
//!
//! # The chain
//!
//! One chain, GNU R's: `R_GlobalContext` (the innermost record) links
//! through `nextcontext` to the session's base record (`R_Toplevel`), with
//! `R_ToplevelContext` and `R_ExitContext` pointing into it. A record lives
//! in the frame of the code that pushed it, at a stable address: a Rust
//! driver's ([`Cx::function_context`], the native entries in `native.rs`)
//! or, for C compiled against robigo's `Defn.h`, a C frame. Its layout is
//! `#[repr(C)]` and checked against `Defn.h`'s ([`check_layout`]).
//!
//! A native entry's jump record is the `cjmpbuf` of its own record (flag
//! `LANDING`): the heap's trampoline sets it, and an export wrapper that
//! leaves C code by an exit `longjmp`s to the innermost such record
//! ([`Cx::prepare_jump`]). Records GNU R has no counterpart of (the native
//! entry of an ALTREP method or a `cend`) are `HIDDEN`: inert in every walk,
//! and skipped by the R-level ones.
//!
//! # Exits
//!
//! An exit is the top record of the pending-exit stack, and an `Err` is
//! returned up the Rust frames (only export wrappers `longjmp`). GNU R's
//! `R_jumpctxt` is split into *legs*: a leg starts where GNU R would call
//! `R_jumpctxt` (when the exit is first seen by the owner of the innermost
//! record, or after an intermediate stop's exit code ran), finds the leg's
//! stop (`first_jump_target`: the first record with `on.exit` code or of
//! type `CTXT_UNWIND`, else the target), and runs the `cend`s of every
//! record above it, innermost first, with `R_GlobalContext` still the
//! innermost, as `R_run_onexits` does before the `longjmp`. The records
//! above the stop are then inert and their owners pop them as the `Err`
//! passes. At the stop, GNU R's globals are restored (`R_restore_globals`),
//! and the owner either lands the exit (the stop is the target) or runs its
//! exit code with the exit *parked* in the record (GNU R's `jumptarget`,
//! `jumpmask` and `R_ReturnedValue`, `context.c:198-244, 298-319`) and
//! resumes it, which starts the next leg. Code that leaves exit code by an
//! exit of its own supersedes the parked one, as in GNU R.
//!
//! Every record is traced: the chain is a precise root (§3, "What a
//! collection marks", item 1), as are the parked exits in it.

#![allow(unsafe_code)]

use crate::exit::{ExitRecord, RError, RResult};
use crate::handle::Ref;
use crate::isolate::{Cx, Isolate};
use crate::root::{Out, Root};
use robigo_heap::unwind::JmpBuf;
use robigo_heap::{Sexp, Trace, Visitor, types};
use std::cell::{Cell, UnsafeCell};
use std::ffi::{c_int, c_void};
use std::mem::MaybeUninit;
use std::ptr::{NonNull, addr_of, addr_of_mut, null_mut};

/// GNU R's context types (`Defn.h`): a record's `callflag`. Only function
/// records have the `FUNCTION` bit.
pub mod ctxt {
    pub const TOPLEVEL: i32 = 0;
    pub const NEXT: i32 = 1;
    pub const BREAK: i32 = 2;
    /// break OR next target
    pub const LOOP: i32 = 3;
    pub const FUNCTION: i32 = 4;
    pub const CCODE: i32 = 8;
    pub const RETURN: i32 = 12;
    pub const BROWSER: i32 = 16;
    pub const GENERIC: i32 = 20;
    pub const RESTART: i32 = 32;
    pub const BUILTIN: i32 = 64;
    pub const UNWIND: i32 = 128;
}

/// robigo's record flags (`Defn.h`, `ROBIGO_CNTXT_*`).
pub(crate) mod rflags {
    /// `cjmpbuf` is set by a running native entry.
    pub const LANDING: u32 = 1;
    /// GNU R has no context here: R-level walks skip the record.
    pub const HIDDEN: u32 = 2;
    /// `promargs` is not filled yet; `nargs` holds its length.
    pub const LAZY_ARGS: u32 = 4;
}

/// GNU R's `R_bcstack_t` union.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union StackValU {
    pub(crate) ival: c_int,
    pub(crate) dval: f64,
    pub(crate) sxpval: Sexp,
}

/// GNU R's `R_bcstack_t`: in a context only `returnValue`, tag 0 with
/// `sxpval` (`SEXP_TO_STACKVAL`; null: undefined).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct StackVal {
    pub(crate) tag: c_int,
    pub(crate) flags: c_int,
    pub(crate) u: StackValU,
}

impl StackVal {
    const NULL: StackVal = StackVal { tag: 0, flags: 0, u: StackValU { sxpval: Sexp::NULL } };
}

/// The filler of a lazy `promargs` (C1): writes the list GNU R's
/// `promiseArgs` would have made for the record's call. Called at most once
/// per record, when something reads `promargs` (`match.call`,
/// `sys.function`'s callers, C that reads the field).
pub type PromargsFill = fn(&mut Cx, ContextId, data: usize, out: Out<'_>) -> RResult<()>;

/// robigo's part of a record (`Defn.h`'s `ROBIGO_CNTXT_EXT`).
#[repr(C)]
pub(crate) struct Ext {
    pub(crate) flags: u32,
    pub(crate) nargs: c_int,
    pub(crate) fillargs: Option<PromargsFill>,
    pub(crate) filldata: usize,
    /// a parked exit: present (0/1), kind, mask, target, value, condition,
    /// call
    pub(crate) parked: u32,
    pub(crate) parkedkind: u32,
    pub(crate) parkedmask: c_int,
    pub(crate) parkedtarget: *mut Context,
    pub(crate) parkedvalue: Sexp,
    pub(crate) parkedcondition: Sexp,
    pub(crate) parkedcall: Sexp,
}

/// GNU R's `RCNTXT` (4.6.2's fields in its order, then robigo's), the
/// layout robigo's `include/Defn.h` declares. Opaque to Rust code above
/// this crate, which names records by [`ContextId`] and reads them through
/// [`CtxRef`].
#[repr(C)]
pub struct Context {
    pub(crate) nextcontext: *mut Context,
    pub(crate) callflag: c_int,
    pub(crate) cjmpbuf: JmpBuf,
    pub(crate) cstacktop: c_int,
    pub(crate) evaldepth: c_int,
    pub(crate) promargs: Sexp,
    pub(crate) callfun: Sexp,
    pub(crate) sysparent: Sexp,
    pub(crate) call: Sexp,
    pub(crate) cloenv: Sexp,
    pub(crate) conexit: Sexp,
    pub(crate) cend: Option<unsafe extern "C" fn(*mut c_void)>,
    pub(crate) cenddata: *mut c_void,
    pub(crate) vmax: *mut c_void,
    pub(crate) intsusp: c_int,
    pub(crate) gcenabled: c_int,
    pub(crate) bcintactive: c_int,
    pub(crate) bcbody: Sexp,
    pub(crate) bcpc: *mut c_void,
    pub(crate) relpc: isize,
    pub(crate) handlerstack: Sexp,
    pub(crate) restartstack: Sexp,
    pub(crate) prstack: *mut c_void,
    pub(crate) nodestack: *mut c_void,
    pub(crate) bcprottop: *mut c_void,
    pub(crate) bcframe: *mut c_void,
    pub(crate) srcref: Sexp,
    pub(crate) browserfinish: c_int,
    pub(crate) return_value: StackVal,
    pub(crate) jumptarget: *mut Context,
    pub(crate) jumpmask: c_int,
    pub(crate) robigo: Ext,
}

// The layout `Defn.h` declares, field by field, in the order of
// `context.c`'s `robigo_rcntxt_layout`.
macro_rules! offsets {
    ($($f:ident)*) => { [$(std::mem::offset_of!(Context, $f)),*] };
}
macro_rules! ext_offsets {
    ($($f:ident)*) => { [$(std::mem::offset_of!(Ext, $f)),*] };
}

/// Rust's view of the layout table `context.c` exports: size, alignment,
/// each field's offset, then `R_bcstack_t`'s size and union offset.
fn rust_layout() -> Vec<usize> {
    let mut v = vec![size_of::<Context>(), align_of::<Context>()];
    v.extend(offsets!(
        nextcontext callflag cjmpbuf cstacktop evaldepth promargs callfun sysparent call cloenv conexit cend
        cenddata vmax intsusp gcenabled bcintactive bcbody bcpc relpc handlerstack restartstack prstack nodestack
        bcprottop bcframe srcref browserfinish return_value jumptarget jumpmask robigo
    ));
    let ext = std::mem::offset_of!(Context, robigo);
    v.extend(
        ext_offsets!(
            flags nargs fillargs filldata parked parkedkind parkedmask parkedtarget parkedvalue parkedcondition parkedcall
        )
        .map(|o| ext + o),
    );
    v.push(size_of::<StackVal>());
    v.push(std::mem::offset_of!(StackVal, u));
    v
}

#[cfg(not(miri))]
unsafe extern "C" {
    static robigo_rcntxt_layout: [usize; 0];
    static robigo_rcntxt_layout_len: usize;
}

/// The layout table of `context.c` (C's `sizeof`, `alignof` and
/// `offsetof`s of `RCNTXT` from `Defn.h`). Miri runs no C: there the
/// Rust table stands in.
pub fn c_layout() -> Vec<usize> {
    #[cfg(not(miri))]
    // SAFETY: `context.c` defines the array with `robigo_rcntxt_layout_len`
    // elements, constant data that nothing writes.
    unsafe {
        std::slice::from_raw_parts(addr_of!(robigo_rcntxt_layout).cast::<usize>(), robigo_rcntxt_layout_len).to_vec()
    }
    #[cfg(miri)]
    rust_layout()
}

/// Check that C (`Defn.h`) and Rust lay `RCNTXT` out alike, size,
/// alignment and every field's offset; panics with the first difference.
/// Run once per isolate and by the crate's tests.
pub fn check_layout() {
    let c = c_layout();
    let r = rust_layout();
    assert_eq!(c.len(), r.len(), "robigo: Defn.h's RCNTXT has another number of fields than context.rs's");
    if let Some(i) = (0..c.len()).find(|&i| c[i] != r[i]) {
        panic!("robigo: RCNTXT's layout differs between Defn.h and context.rs at entry {i}: C {}, Rust {}", c[i], r[i]);
    }
}

// The sizes the hot paths rely on: a record handle is a word, and the
// record is GNU R's plus robigo's 72 bytes (x86-64: 432 + 72).
const _: () = {
    assert!(size_of::<ContextId>() == 8);
    assert!(size_of::<Option<ContextId>>() == 8);
    assert!(size_of::<Rec>() == 8);
    assert!(size_of::<StackVal>() == 16);
    assert!(size_of::<Ext>() == 72);
};

/// A record of the chain, named by address: what an exit targets and a
/// driver hands its body. `Copy`; reading it goes through [`Cx::context`]
/// (which checks that it is still on the chain) or the driver that owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContextId(NonNull<Context>);

impl ContextId {
    /// The record's address (an exit's target in [`crate::ExitInfo`]).
    pub fn addr(self) -> usize {
        self.0.as_ptr() as usize
    }

    /// The id of the record at `p` (an `RCNTXT *` from C), or None for
    /// null. Nothing is read: [`Cx::context`] checks it is on the chain.
    pub fn from_ptr(p: *mut Context) -> Option<ContextId> {
        NonNull::new(p).map(ContextId)
    }
}

/// A live record, by raw pointer: the crate's only way to read or write a
/// record. Invariant: points to a record that is initialized and live (on
/// the chain, or being pushed or popped by its owner), and nothing holds a
/// Rust reference into it (C and the collector read and write records
/// through pointers too).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rec(pub(crate) NonNull<Context>);

macro_rules! rec_fields {
    ($($get:ident $set:ident: $t:ty;)*) => {
        $(
            #[inline(always)]
            pub(crate) fn $get(self) -> $t {
                // SAFETY: the type's invariant: a live, initialized record
                // that no reference aliases.
                unsafe { addr_of!((*self.0.as_ptr()).$get).read() }
            }
            #[inline(always)]
            pub(crate) fn $set(self, v: $t) {
                // SAFETY: as in the getter.
                unsafe { addr_of_mut!((*self.0.as_ptr()).$get).write(v) }
            }
        )*
    };
}

#[allow(dead_code)]
impl Rec {
    rec_fields! {
        nextcontext set_nextcontext: *mut Context;
        callflag set_callflag: c_int;
        cstacktop set_cstacktop: c_int;
        evaldepth set_evaldepth: c_int;
        promargs set_promargs: Sexp;
        callfun set_callfun: Sexp;
        sysparent set_sysparent: Sexp;
        call set_call: Sexp;
        cloenv set_cloenv: Sexp;
        conexit set_conexit: Sexp;
        cend set_cend: Option<unsafe extern "C" fn(*mut c_void)>;
        cenddata set_cenddata: *mut c_void;
        intsusp set_intsusp: c_int;
        handlerstack set_handlerstack: Sexp;
        restartstack set_restartstack: Sexp;
        srcref set_srcref: Sexp;
        browserfinish set_browserfinish: c_int;
        jumptarget set_jumptarget: *mut Context;
        jumpmask set_jumpmask: c_int;
    }

    #[inline(always)]
    pub(crate) fn ptr(self) -> *mut Context {
        self.0.as_ptr()
    }

    #[inline(always)]
    pub(crate) fn id(self) -> ContextId {
        ContextId(self.0)
    }

    #[inline(always)]
    fn ext(self) -> *mut Ext {
        // SAFETY: the invariant; a field address, no reference made.
        unsafe { addr_of_mut!((*self.0.as_ptr()).robigo) }
    }

    #[inline(always)]
    pub(crate) fn rflags(self) -> u32 {
        // SAFETY: the invariant.
        unsafe { addr_of!((*self.ext()).flags).read() }
    }

    #[inline(always)]
    pub(crate) fn set_rflags(self, f: u32) {
        // SAFETY: the invariant.
        unsafe { addr_of_mut!((*self.ext()).flags).write(f) }
    }

    #[inline(always)]
    pub(crate) fn landing(self) -> bool {
        self.rflags() & rflags::LANDING != 0
    }

    #[inline(always)]
    pub(crate) fn hidden(self) -> bool {
        self.rflags() & rflags::HIDDEN != 0
    }

    pub(crate) fn nargs(self) -> c_int {
        // SAFETY: the invariant.
        unsafe { addr_of!((*self.ext()).nargs).read() }
    }

    pub(crate) fn fill(self) -> (Option<PromargsFill>, usize) {
        // SAFETY: the invariant.
        unsafe { (addr_of!((*self.ext()).fillargs).read(), addr_of!((*self.ext()).filldata).read()) }
    }

    /// `returnValue`'s object (null: undefined).
    pub(crate) fn return_value(self) -> Sexp {
        // SAFETY: the invariant; the record's returnValue is always tag 0
        // (written only by `set_return_value` and `begin`).
        unsafe { addr_of!((*self.0.as_ptr()).return_value.u.sxpval).read() }
    }

    pub(crate) fn set_return_value(self, v: Sexp) {
        // SAFETY: the invariant.
        unsafe {
            addr_of_mut!((*self.0.as_ptr()).return_value)
                .write(StackVal { u: StackValU { sxpval: v }, ..StackVal::NULL })
        }
    }

    /// The jump record, for the native entry that sets it.
    pub(crate) fn jmpbuf(self) -> *mut JmpBuf {
        // SAFETY: the invariant; a field address.
        unsafe { addr_of_mut!((*self.0.as_ptr()).cjmpbuf) }
    }

    fn parked(self) -> Option<ExitRecord> {
        let x = self.ext();
        // SAFETY: the invariant.
        unsafe {
            if addr_of!((*x).parked).read() == 0 {
                return None;
            }
            Some(ExitRecord {
                info: crate::ExitInfo {
                    kind: crate::exit::kind_from_u8(addr_of!((*x).parkedkind).read() as u8),
                    mask: addr_of!((*x).parkedmask).read(),
                    target: addr_of!((*x).parkedtarget).read() as usize,
                },
                value: addr_of!((*x).parkedvalue).read(),
                condition: addr_of!((*x).parkedcondition).read(),
                call: addr_of!((*x).parkedcall).read(),
                stop: 0,
            })
        }
    }

    fn set_parked(self, r: Option<ExitRecord>) {
        let x = self.ext();
        // SAFETY: the invariant.
        unsafe {
            match r {
                None => {
                    addr_of_mut!((*x).parked).write(0);
                    addr_of_mut!((*x).parkedtarget).write(null_mut());
                    addr_of_mut!((*x).parkedvalue).write(Sexp::NULL);
                    addr_of_mut!((*x).parkedcondition).write(Sexp::NULL);
                    addr_of_mut!((*x).parkedcall).write(Sexp::NULL);
                }
                Some(r) => {
                    addr_of_mut!((*x).parked).write(1);
                    addr_of_mut!((*x).parkedkind).write(r.info.kind as u32);
                    addr_of_mut!((*x).parkedmask).write(r.info.mask);
                    addr_of_mut!((*x).parkedtarget).write(r.info.target as *mut Context);
                    addr_of_mut!((*x).parkedvalue).write(r.value);
                    addr_of_mut!((*x).parkedcondition).write(r.condition);
                    addr_of_mut!((*x).parkedcall).write(r.call);
                }
            }
        }
    }

    /// Every object edge of the record, for a collection (and
    /// [`Cx::for_each_root`]).
    ///
    /// # Safety
    /// Nothing else reads or writes the record meanwhile.
    unsafe fn trace_edges<V: Visitor + ?Sized>(self, v: &mut V) {
        let p = self.0.as_ptr();
        // SAFETY: the caller's contract and the invariant: the record's
        // object fields hold null, immortal or live objects, and no other
        // reference to them exists while these are made.
        unsafe {
            v.edge(&mut *addr_of_mut!((*p).promargs));
            v.edge(&mut *addr_of_mut!((*p).callfun));
            v.edge(&mut *addr_of_mut!((*p).sysparent));
            v.edge(&mut *addr_of_mut!((*p).call));
            v.edge(&mut *addr_of_mut!((*p).cloenv));
            v.edge(&mut *addr_of_mut!((*p).conexit));
            v.edge(&mut *addr_of_mut!((*p).bcbody));
            v.edge(&mut *addr_of_mut!((*p).handlerstack));
            v.edge(&mut *addr_of_mut!((*p).restartstack));
            v.edge(&mut *addr_of_mut!((*p).srcref));
            if (*p).return_value.tag == 0 {
                v.edge(&mut *addr_of_mut!((*p).return_value.u.sxpval));
            }
            let x = addr_of_mut!((*p).robigo);
            v.edge(&mut *addr_of_mut!((*x).parkedvalue));
            v.edge(&mut *addr_of_mut!((*x).parkedcondition));
            v.edge(&mut *addr_of_mut!((*x).parkedcall));
        }
    }
}

/// The isolate's context state: the chain's ends and the globals GNU R
/// saves in a record when it is pushed and restores when an exit stops at
/// it (`begincontext`, `R_restore_globals`). Part of the isolate's roots
/// (its `Trace` walks the chain); every field is a `Cell`, read and written
/// one call at a time.
pub(crate) struct Contexts {
    /// R_GlobalContext
    pub(crate) global: Cell<*mut Context>,
    /// R_ToplevelContext
    pub(crate) toplevel: Cell<*mut Context>,
    /// R_ExitContext (null: none)
    pub(crate) exit_context: Cell<*mut Context>,
    /// R_Toplevel, the session's base record (freed with the isolate)
    base: NonNull<Context>,
    /// R_NilValue (immortal: the root list leaves it out)
    nil: Sexp,
    /// R_HandlerStack, R_RestartStack (S1 fills them), R_Srcref
    pub(crate) handler_stack: Cell<Sexp>,
    pub(crate) restart_stack: Cell<Sexp>,
    pub(crate) srcref: Cell<Sexp>,
    /// R_Visible
    pub(crate) visible: Cell<bool>,
    /// R_EvalDepth, R_Expressions, R_Expressions_keep
    pub(crate) eval_depth: Cell<i32>,
    pub(crate) expressions: Cell<i32>,
    pub(crate) expressions_keep: Cell<i32>,
    /// R_CStackStart, R_CStackLimit (usize::MAX: -1, none), R_CStackDir,
    /// R_OldCStackLimit
    pub(crate) cstack_start: usize,
    pub(crate) cstack_limit: Cell<usize>,
    pub(crate) cstack_dir: isize,
    pub(crate) old_cstack_limit: Cell<usize>,
    /// R_interrupts_suspended, R_interrupts_pending
    pub(crate) intr_suspended: Cell<bool>,
    pub(crate) intr_pending: Cell<bool>,
    /// Fill `promargs` when a record is pushed (compiled C that reads the
    /// field is loaded, docs/g3.md §6.5), not when it is first read.
    pub(crate) eager_promargs: Cell<bool>,
    /// `eval`'s and `bcEval`'s `evalcount` (eval.c:1085, eval.c:7538): the
    /// two polls of GNU R's unit, per isolate (docs/g3.md §3 item 7).
    pub(crate) evalcount: Cell<i32>,
    pub(crate) bc_evalcount: Cell<i32>,
}

impl Contexts {
    /// The state of a new session (`setup_Rmainloop`): the base record
    /// alone on the chain, `nil` everywhere, GNU R's default limits, the C
    /// stack of the thread the isolate is made on (`stack`: lowest,
    /// highest address).
    pub(crate) fn new(nil: Sexp, stack: (usize, usize)) -> Contexts {
        check_layout();
        let base = NonNull::from(Box::leak(Box::new(blank_context(nil))));
        // R: main.c:setup_Rmainloop
        let b = Rec(base);
        b.set_callflag(ctxt::TOPLEVEL);
        // R: unix/system.c (the stack's size) and main.c:879-881
        let (lo, hi) = stack;
        let mut limit = hi.wrapping_sub(lo);
        if limit == 0 || limit > 100_000_000 {
            limit = usize::MAX;
        }
        if limit != usize::MAX {
            limit = (0.95 * limit as f64) as usize;
        }
        Contexts {
            global: Cell::new(base.as_ptr()),
            toplevel: Cell::new(base.as_ptr()),
            exit_context: Cell::new(null_mut()),
            base,
            nil,
            handler_stack: Cell::new(nil),
            restart_stack: Cell::new(nil),
            srcref: Cell::new(nil),
            visible: Cell::new(true),
            eval_depth: Cell::new(0),
            expressions: Cell::new(5000),
            expressions_keep: Cell::new(5000),
            cstack_start: hi,
            cstack_limit: Cell::new(limit),
            cstack_dir: 1,
            old_cstack_limit: Cell::new(0),
            intr_suspended: Cell::new(false),
            intr_pending: Cell::new(false),
            eager_promargs: Cell::new(false),
            evalcount: Cell::new(0),
            bc_evalcount: Cell::new(0),
        }
    }

    #[inline(always)]
    pub(crate) fn base(&self) -> Rec {
        Rec(self.base)
    }

    /// Every record from the innermost to the base, hidden ones included.
    pub(crate) fn chain(&self) -> impl Iterator<Item = Rec> + use<> {
        let mut c = self.global.get();
        std::iter::from_fn(move || {
            let r = Rec(NonNull::new(c)?);
            c = r.nextcontext();
            Some(r)
        })
    }

    /// Every object the context state reaches: the globals, then each
    /// record's fields, innermost first (the same edges as its `Trace`),
    /// R_NilValue left out (it is immortal, and every idle field holds it).
    pub(crate) fn for_each_object(&self, f: &mut dyn FnMut(Sexp)) {
        let nil = self.nil;
        let mut f = |x: Sexp| {
            if x != nil {
                f(x)
            }
        };
        f(self.handler_stack.get());
        f(self.restart_stack.get());
        f(self.srcref.get());
        for r in self.chain() {
            let mut v = Collect(&mut f);
            // SAFETY: nothing else touches the record during the call (the
            // collector only uses the edges it is shown, and `f` cannot pop
            // a record: that takes the owner's `&mut Cx`).
            unsafe { r.trace_edges(&mut v) };
        }
    }
}

/// A visitor that hands each edge's object to a closure.
struct Collect<'f>(&'f mut dyn FnMut(Sexp));

impl Visitor for Collect<'_> {
    fn edge(&mut self, x: &mut Sexp) {
        if !x.is_null() {
            (self.0)(*x);
        }
    }
}

impl Drop for Contexts {
    fn drop(&mut self) {
        // SAFETY: made by `Box::leak` in `new` and freed once, here; the
        // isolate is going, so no record links to it any more.
        drop(unsafe { Box::from_raw(self.base.as_ptr()) });
    }
}

/// The chain is a precise root: every record's object fields, innermost
/// first, and the globals; a moving collection rewrites them in place.
impl Trace for Contexts {
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        let mut x = self.handler_stack.get();
        v.edge(&mut x);
        self.handler_stack.set(x);
        let mut x = self.restart_stack.get();
        v.edge(&mut x);
        self.restart_stack.set(x);
        let mut x = self.srcref.get();
        v.edge(&mut x);
        self.srcref.set(x);
        for r in self.chain() {
            // SAFETY: a collection has the isolate's roots exclusively, and
            // records are read and written only by their owners, C code and
            // collections, never at the same time as one (no record is
            // touched while the collector runs).
            unsafe { r.trace_edges(v) };
        }
    }
}

/// A record with every field GNU R's `begincontext` writes at its blank
/// value: null pointers, `nil` objects, no jump record.
fn blank_context(nil: Sexp) -> Context {
    Context {
        nextcontext: null_mut(),
        callflag: ctxt::TOPLEVEL,
        cjmpbuf: JmpBuf::UNSET,
        cstacktop: 0,
        evaldepth: 0,
        promargs: nil,
        callfun: nil,
        sysparent: nil,
        call: nil,
        cloenv: nil,
        conexit: nil,
        cend: None,
        cenddata: null_mut(),
        vmax: null_mut(),
        intsusp: 0,
        gcenabled: 1,
        bcintactive: 0,
        bcbody: nil,
        bcpc: null_mut(),
        relpc: 0,
        handlerstack: nil,
        restartstack: nil,
        prstack: null_mut(),
        nodestack: null_mut(),
        bcprottop: null_mut(),
        bcframe: null_mut(),
        srcref: nil,
        browserfinish: 0,
        return_value: StackVal::NULL,
        jumptarget: null_mut(),
        jumpmask: 0,
        robigo: Ext {
            flags: 0,
            nargs: 0,
            fillargs: None,
            filldata: 0,
            parked: 0,
            parkedkind: 0,
            parkedmask: 0,
            parkedtarget: null_mut(),
            parkedvalue: Sexp::NULL,
            parkedcondition: Sexp::NULL,
            parkedcall: Sexp::NULL,
        },
    }
}

/// What a pushed record holds (`begincontext`'s arguments), as raw objects
/// read from roots in the same `&Cx` interval that pushes it.
pub(crate) struct Init {
    pub(crate) nil: Sexp,
    pub(crate) callflag: i32,
    pub(crate) rflags: u32,
    pub(crate) call: Sexp,
    pub(crate) cloenv: Sexp,
    pub(crate) sysparent: Sexp,
    pub(crate) promargs: Sexp,
    pub(crate) callfun: Sexp,
    pub(crate) nargs: c_int,
    pub(crate) fill: Option<PromargsFill>,
    pub(crate) filldata: usize,
}

impl Init {
    /// A record of type `callflag` with `nil` in every object field.
    pub(crate) fn plain(cx: &Cx, callflag: i32, rflags: u32) -> Init {
        let nil = cx.iso().nil;
        Init {
            nil,
            callflag,
            rflags,
            call: nil,
            cloenv: nil,
            sysparent: nil,
            promargs: nil,
            callfun: nil,
            nargs: 0,
            fill: None,
            filldata: 0,
        }
    }
}

/// Where an exit stands when it reaches the owner of the innermost record.
pub(crate) enum Arrival {
    /// The record is above this leg's stop: the exit passes it (the owner
    /// pops the record and returns the `Err`).
    Pass(RError),
    /// The record is the exit's target: GNU R's globals are restored; the
    /// owner resolves it.
    Landed(RError),
    /// The record is this leg's stop and not its target (it has `on.exit`
    /// code, or is an `R_UnwindProtect` record): the owner runs its exit
    /// code with the exit parked, then resumes it.
    Stop(RError),
}

/// What [`Cx::with_context`] reports when its record did not end normally.
#[derive(Debug)]
pub enum Left {
    /// An exit targeting the record landed there and was resolved (a
    /// `break`, a `next`, a return to a browser or restart record): its
    /// mask, and its value written to the `Out` given.
    Landed(crate::ExitInfo),
    /// An exit passed the record and continues.
    Exit(RError),
}

/// How a function record's body is described to [`Cx::function_context`]:
/// `begincontext`'s arguments (`R462/main/eval.c:R_execClosure`).
#[derive(Clone, Copy)]
pub struct FunctionContext<'s> {
    /// `CTXT_RETURN` for a closure call; `CTXT_GENERIC` for a method
    /// dispatch (`usemethod`), and so on: any type with the
    /// [`ctxt::FUNCTION`] bit.
    pub callflag: i32,
    pub call: Root<'s>,
    /// The new frame (`newrho`).
    pub cloenv: Root<'s>,
    /// The caller's frame (`sysparent`).
    pub sysparent: Root<'s>,
    /// The closure (`op`).
    pub callfun: Root<'s>,
    pub promargs: Promargs<'s>,
}

/// A function record's `promargs`.
#[derive(Clone, Copy)]
pub enum Promargs<'s> {
    /// Already a list (C called the closure with one, or it is cheap).
    Filled(Root<'s>),
    /// Made when first read (docs/g3.md §6.5): `nargs` is its length (what
    /// `nargs()` reports without making it), `fill` makes it.
    Lazy { nargs: i32, fill: PromargsFill, data: usize },
}

/// [`FunctionContext`] as raw objects ([`Cx::function_context_raw`]);
/// `promargs` is the list, or (`nargs`, filler, data) for a lazy one.
#[derive(Clone, Copy, Debug)]
pub struct RawFunctionContext {
    pub callflag: i32,
    pub call: Sexp,
    pub cloenv: Sexp,
    pub sysparent: Sexp,
    pub callfun: Sexp,
    pub promargs: Result<Sexp, (i32, PromargsFill, usize)>,
}

/// The evaluator of `on.exit` code, which the record's owner supplies
/// (V1's `eval`; robigo-rtcore has none): evaluate `expr` in `env`,
/// discarding the value.
pub type OnExitEval<'a> = dyn FnMut(&mut Cx, Root<'_>, Root<'_>) -> RResult<()> + 'a;

/// A read view of a live record for one `&'g Cx` interval: a record
/// cannot be popped while `&Cx` is held (popping takes its owner's
/// `&mut Cx`), and nothing collects.
#[derive(Clone, Copy)]
pub struct CtxRef<'g> {
    rec: Rec,
    cx: &'g Cx,
}

impl<'g> CtxRef<'g> {
    #[inline]
    fn obj(self, x: Sexp) -> Ref<'g> {
        // SAFETY: a record's object fields are traced (the chain is a
        // precise root) and nothing collects during 'g.
        unsafe { Ref::from_sexp(x, self.cx) }
    }

    pub fn id(self) -> ContextId {
        self.rec.id()
    }
    /// GNU R's `callflag`.
    pub fn callflag(self) -> i32 {
        self.rec.callflag()
    }
    pub fn call(self) -> Ref<'g> {
        self.obj(self.rec.call())
    }
    pub fn callfun(self) -> Ref<'g> {
        self.obj(self.rec.callfun())
    }
    pub fn cloenv(self) -> Ref<'g> {
        self.obj(self.rec.cloenv())
    }
    pub fn sysparent(self) -> Ref<'g> {
        self.obj(self.rec.sysparent())
    }
    /// The `on.exit` code still to run (a pairlist of expressions).
    pub fn conexit(self) -> Ref<'g> {
        self.obj(self.rec.conexit())
    }
    pub fn srcref(self) -> Ref<'g> {
        self.obj(self.rec.srcref())
    }
    pub fn handler_stack(self) -> Ref<'g> {
        self.obj(self.rec.handlerstack())
    }
    pub fn restart_stack(self) -> Ref<'g> {
        self.obj(self.rec.restartstack())
    }
    /// `promargs`, or None while it is lazy ([`Cx::materialize_promargs`]
    /// makes it).
    pub fn promargs(self) -> Option<Ref<'g>> {
        (self.rec.rflags() & rflags::LAZY_ARGS == 0).then(|| self.obj(self.rec.promargs()))
    }
    /// `length(promargs)`, without making it (`do_nargs`).
    pub fn nargs(self) -> i32 {
        if self.rec.rflags() & rflags::LAZY_ARGS != 0 { self.rec.nargs() } else { list_length(self.rec.promargs()) }
    }
    /// The value an exit carried to the record, or its body's (GNU R's
    /// `returnValue`), while its `on.exit` code runs; None when undefined
    /// (an intermediate stop).
    pub fn return_value(self) -> Option<Ref<'g>> {
        let v = self.rec.return_value();
        (!v.is_null()).then(|| self.obj(v))
    }
    /// The next record as R code sees the chain (hidden ones skipped).
    pub fn next(self) -> Option<CtxRef<'g>> {
        let mut c = self.rec.nextcontext();
        loop {
            let r = Rec(NonNull::new(c)?);
            if !r.hidden() {
                return Some(CtxRef { rec: r, cx: self.cx });
            }
            c = r.nextcontext();
        }
    }
    pub(crate) fn rec(self) -> Rec {
        self.rec
    }
}

impl std::fmt::Debug for CtxRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CtxRef({:p}, callflag {})", self.rec.ptr(), self.rec.callflag())
    }
}

/// `length()` of a pairlist (GNU R's for LISTSXP/LANGSXP/DOTSXP; 0 for
/// NULL).
pub(crate) fn list_length(mut x: Sexp) -> i32 {
    let mut n = 0;
    // SAFETY: `x` is a live pairlist from a record (traced), its cells
    // live with it; nothing collects during the walk.
    unsafe {
        while matches!(x.typ(), types::LISTSXP | types::LANGSXP | types::DOTSXP) {
            n += 1;
            x = x.cdr();
        }
    }
    n
}

/// Unlinks a driver's record on every path: normally after the driver's
/// exit code ran, or if a panic unwinds through the driver. The record is
/// the innermost one then (inner drivers unlinked theirs first).
struct Unlink {
    iso: NonNull<Isolate>,
    rec: Rec,
}

impl Drop for Unlink {
    fn drop(&mut self) {
        // SAFETY: made by `Cx::with_record`, whose `&mut Cx` borrow keeps the
        // isolate alive until this guard is dropped.
        let st = unsafe { self.iso.as_ref() }.contexts();
        debug_assert!(
            st.global.get() == self.rec.ptr() || std::thread::panicking(),
            "robigo: a context left records above it on the chain"
        );
        st.global.set(self.rec.nextcontext());
        if st.exit_context.get() == self.rec.ptr() {
            st.exit_context.set(null_mut());
        }
    }
}

impl Cx {
    /// The isolate's context state.
    #[inline(always)]
    pub(crate) fn ctxs(&self) -> &Contexts {
        self.iso().contexts()
    }

    /// Push a record built from `init` in this frame, run `f` with it, and
    /// pop it on every path (`begincontext`, `endcontext`'s last step). The
    /// record stays at its address while `f` runs.
    pub(crate) fn with_record<R>(&mut self, init: &Init, f: impl FnOnce(&mut Cx, Rec) -> R) -> R {
        let cell = UnsafeCell::new(MaybeUninit::<Context>::uninit());
        // SAFETY: an UnsafeCell's pointer is never null.
        let rec = Rec(unsafe { NonNull::new_unchecked(cell.get().cast::<Context>()) });
        self.begin(rec, init);
        let _pop = Unlink { iso: self.iso_ptr(), rec };
        f(self, rec)
    }

    /// GNU R's `begincontext`: write every field of `rec` (uninitialized
    /// memory) and link it as the innermost record.
    fn begin(&mut self, rec: Rec, init: &Init) {
        let st = self.ctxs();
        let global = st.global.get();
        let p = rec.ptr();
        // SAFETY: `global` is the live innermost record.
        let browserfinish = Rec(unsafe { NonNull::new_unchecked(global) }).browserfinish();
        // R: context.c:begincontext, field by field in place: every field
        // but the jump record, which only a native entry sets and reads.
        // (cstacktop, gcenabled, vmax and the byte-code fields are A1's
        // protect and R_alloc stacks, the heap's GC switch and V1's VM
        // state: they save theirs here as they land.)
        // SAFETY: `p` points to memory for a record that nothing else uses
        // yet; each write initializes one field.
        unsafe {
            macro_rules! w {
                ($($f:ident = $v:expr;)*) => { $(addr_of_mut!((*p).$f).write($v);)* };
            }
            w! {
                nextcontext = global;
                callflag = init.callflag;
                cstacktop = 0;
                evaldepth = st.eval_depth.get();
                promargs = init.promargs;
                callfun = init.callfun;
                sysparent = init.sysparent;
                call = init.call;
                cloenv = init.cloenv;
                conexit = init.nil;
                cend = None;
                cenddata = null_mut();
                vmax = null_mut();
                intsusp = st.intr_suspended.get() as c_int;
                gcenabled = 1;
                bcintactive = 0;
                bcbody = init.nil;
                bcpc = null_mut();
                relpc = 0;
                handlerstack = st.handler_stack.get();
                restartstack = st.restart_stack.get();
                prstack = null_mut();
                nodestack = null_mut();
                bcprottop = null_mut();
                bcframe = null_mut();
                srcref = st.srcref.get();
                browserfinish = browserfinish;
                return_value = StackVal::NULL;
                jumptarget = null_mut();
                jumpmask = 0;
            }
            addr_of_mut!((*p).robigo).write(Ext {
                flags: init.rflags,
                nargs: init.nargs,
                fillargs: init.fill,
                filldata: init.filldata,
                parked: 0,
                parkedkind: 0,
                parkedmask: 0,
                parkedtarget: null_mut(),
                parkedvalue: Sexp::NULL,
                parkedcondition: Sexp::NULL,
                parkedcall: Sexp::NULL,
            });
        }
        st.global.set(p);
    }

    /// GNU R's `R_restore_globals` for the parts robigo has (the protect
    /// and R_alloc stacks, the GC switch, pending promises and the VM state
    /// join as A1, C1 and V1 add them), and `R_jumpctxt`'s C stack limit:
    /// what an exit restores at its stop.
    pub(crate) fn restore_globals(&mut self, c: Rec) {
        let st = self.ctxs();
        // R: context.c:R_restore_globals
        st.eval_depth.set(c.evaldepth());
        st.intr_suspended.set(c.intsusp() != 0);
        st.handler_stack.set(c.handlerstack());
        st.restart_stack.set(c.restartstack());
        st.expressions.set(st.expressions_keep.get());
        st.srcref.set(c.srcref());
        // R: context.c:R_jumpctxt
        if st.old_cstack_limit.get() != 0 {
            st.cstack_limit.set(st.old_cstack_limit.get());
            st.old_cstack_limit.set(0);
        }
    }

    /// GNU R's `R_UnwindHandlerStack(target)`, as `endcontext` uses it: pop
    /// the handler stack back to `target` if `target` is on it, clearing
    /// the environments of the entries it pops; otherwise `target` is a
    /// saved stack to restore.
    #[inline(always)]
    pub(crate) fn unwind_handler_stack(&mut self, target: Sexp) -> Sexp {
        // (the usual case: nothing to pop)
        if self.ctxs().handler_stack.get() == target { target } else { self.unwind_handler_stack_slow(target) }
    }

    #[cold]
    #[inline(never)]
    fn unwind_handler_stack_slow(&mut self, target: Sexp) -> Sexp {
        let st = self.ctxs();
        let nil = self.iso().nil;
        // R: errors.c:R_UnwindHandlerStack
        let mut hs = st.handler_stack.get();
        // SAFETY: the handler stack is a traced pairlist of live entries
        // (VECSXPs of 5, `ENTRY_*` in errors.c), as S1 builds them.
        unsafe {
            while hs != target && hs != nil {
                hs = hs.cdr();
            }
            if hs != target {
                return target;
            }
            let mut hs = st.handler_stack.get();
            while hs != target {
                st.handler_stack.set(hs.cdr());
                let e = hs.car();
                if e.typ() == types::VECSXP && e.std_len() >= 5 {
                    e.set_elt(1, nil);
                    e.set_elt(3, nil);
                }
                hs = hs.cdr();
            }
        }
        target
    }

    // ---- exits through the chain --------------------------------------

    /// The top record of the pending-exit stack.
    pub(crate) fn top_exit(&self) -> ExitRecord {
        let iso = self.iso();
        iso.exit_count()
            .checked_sub(1)
            .and_then(|i| iso.exit_at(i))
            .expect("robigo: an RError without its pending-exit record")
    }

    /// Start the current leg of the pending exit `e`, unless it is under
    /// way: `R_jumpctxt` up to its `LONGJMP`. Finds the leg's stop
    /// (`first_jump_target`) from the innermost record, records it in the
    /// exit, and runs every `cend` above it (`R_run_onexits`). A `cend`
    /// that leaves by an exit of its own supersedes `e`: then that exit
    /// (with its own leg started by its jumper) is returned.
    pub(crate) fn start_leg(&mut self, e: RError) -> RError {
        let rec = self.top_exit();
        if rec.stop != 0 {
            return e;
        }
        let target = rec.info.target as *mut Context;
        let mask = rec.info.mask;
        let st = self.ctxs();
        let nil = self.iso().nil;
        // R: context.c:first_jump_target
        let mut stop = target;
        for c in st.chain() {
            if c.ptr() == target {
                break;
            }
            if (c.cloenv() != nil && c.conexit() != nil) || c.callflag() == ctxt::UNWIND {
                c.set_jumptarget(target);
                c.set_jumpmask(mask);
                stop = c.ptr();
                break;
            }
        }
        self.iso().exit_set_stop(stop as usize);
        // R: context.c:R_jumpctxt
        let savevis = st.visible.get();
        match self.run_onexits(stop, e) {
            Ok(e) => {
                self.ctxs().visible.set(savevis);
                e
            }
            // (a superseding exit's own leg restored R_Visible; GNU R
            // never returns to this R_jumpctxt)
            Err(e2) => e2,
        }
    }

    /// GNU R's `R_run_onexits(stop)` for an exit's leg: the `cend` of every
    /// record from the innermost down to `stop` (excluded), each called
    /// with the exit parked in its record. The records above a leg's stop
    /// carry no `on.exit` code: `first_jump_target` stopped at the first
    /// that does. Err: a `cend` left by an exit of its own, which
    /// supersedes `e` (its leg is under way).
    fn run_onexits(&mut self, stop: *mut Context, mut e: RError) -> Result<RError, RError> {
        let nil = self.iso().nil;
        let mut c = self.ctxs().global.get();
        while c != stop {
            let Some(r) = NonNull::new(c).map(Rec) else {
                // R: context.c:R_run_onexits (a target that is not on the
                // chain)
                self.drop_exit(e);
                let e2 = self.error_call_sexp(nil, "bad target context--should NEVER happen if R was called correctly");
                return Err(self.start_leg(e2));
            };
            if let Some(cend) = r.cend() {
                r.set_cend(None); // prevent recursion
                let st = self.ctxs();
                st.handler_stack.set(r.handlerstack());
                st.restart_stack.set(r.restartstack());
                self.park(r, e);
                match self.call_cend(cend, r.cenddata()) {
                    Ok(()) => e = self.unpark_in_leg(r, stop as usize),
                    Err(e2) => {
                        // the cend left by its own exit: it supersedes
                        r.set_parked(None);
                        return Err(e2);
                    }
                }
            }
            debug_assert!(r.cloenv() == nil || r.conexit() == nil, "robigo: on.exit code above a leg's stop");
            let st = self.ctxs();
            if st.exit_context.get() == c {
                st.exit_context.set(null_mut());
            }
            c = r.nextcontext();
        }
        Ok(e)
    }

    /// An exit reached the owner of `c`, the innermost record: start its
    /// leg if needed, and say what `c` is to it. At the leg's stop, GNU R's
    /// globals are restored from `c` (`R_jumpctxt`, before its `LONGJMP`).
    pub(crate) fn arrive(&mut self, c: Rec, e: RError) -> Arrival {
        debug_assert!(
            self.ctxs().global.get() == c.ptr(),
            "robigo: an exit reached a record that is not the innermost"
        );
        let e = self.start_leg(e);
        let rec = self.top_exit();
        if rec.stop != c.ptr() as usize {
            return Arrival::Pass(e);
        }
        self.restore_globals(c);
        if rec.info.target == c.ptr() as usize { Arrival::Landed(e) } else { Arrival::Stop(e) }
    }

    /// Move the pending exit `e` into `c` while `c`'s exit code runs.
    pub(crate) fn park(&mut self, c: Rec, e: RError) {
        let RError { .. } = e;
        debug_assert!(c.parked().is_none(), "robigo: two exits parked in one context");
        let rec = self.iso().exit_pop().expect("robigo: an RError without its pending-exit record");
        c.set_parked(Some(rec));
    }

    /// Resume the exit parked in `c`: it is pending again, with a new leg
    /// to start (GNU R's `endcontext` calls `R_jumpctxt` again).
    pub(crate) fn unpark(&mut self, c: Rec) -> RError {
        let rec = c.parked().expect("robigo: no exit parked in the context");
        c.set_parked(None);
        self.iso().exit_push(rec);
        RError::new()
    }

    /// Resume the exit parked in `c` where its leg stood (the exit code
    /// ran within the leg: a `cend` during `R_run_onexits`).
    fn unpark_in_leg(&mut self, c: Rec, stop: usize) -> RError {
        let e = self.unpark(c);
        self.iso().exit_set_stop(stop);
        e
    }

    /// Pop the pending exit `e` and drop it (superseded).
    pub(crate) fn drop_exit(&mut self, e: RError) {
        let RError { .. } = e;
        self.iso().exit_pop().expect("robigo: an RError without its pending-exit record");
    }

    /// Pop the pending exit `e`, returning its record (a landing takes the
    /// value).
    pub(crate) fn take_exit(&mut self, e: RError) -> ExitRecord {
        let RError { .. } = e;
        self.iso().exit_pop().expect("robigo: an RError without its pending-exit record")
    }

    /// Call a `cend` (`void (*)(void *)`) through a hidden native entry, so
    /// that an exit out of it comes back as an `Err`.
    fn call_cend(&mut self, cend: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> RResult<()> {
        // SAFETY: a `cend` takes one pointer, its `cenddata`, and returns
        // nothing (GNU R's contract for `RCNTXT::cend`).
        unsafe { self.native6_hidden(cend as *const c_void, [data as usize, 0, 0, 0, 0, 0]) }.map(|_| ())
    }

    // ---- function records ------------------------------------------------

    /// Run `body` as a closure call in a function record (`callflag` with
    /// the `FUNCTION` bit), the way `R_execClosure` does: the body's value,
    /// or the value of a `return` that lands here, is the record's
    /// `returnValue` while its `on.exit` code runs (evaluated with `eval`),
    /// and is written to `out` at the end.
    ///
    /// An exit passing the record with `on.exit` code to run stops here:
    /// the code runs with the exit parked, then the exit continues, unless
    /// the code leaves by an exit of its own, which supersedes it (a
    /// `return` in `on.exit` code lands here and returns normally, as in
    /// GNU R).
    pub fn function_context(
        &mut self,
        spec: &FunctionContext<'_>,
        out: Out<'_>,
        eval: &mut OnExitEval<'_>,
        body: impl FnOnce(&mut Cx, ContextId, Out<'_>) -> RResult<()>,
    ) -> RResult<()> {
        debug_assert!(spec.callflag & ctxt::FUNCTION != 0, "robigo: a function record without the FUNCTION bit");
        let init = self.function_init(spec);
        self.run_function(init, out, eval, body)
    }

    /// [`Cx::function_context`] for the raw layer (the VM), whose call,
    /// frames, closure and `promargs` are objects in its own rooted slots:
    /// no root is bound or checked.
    ///
    /// # Safety
    /// Every object in `spec` is a live object of this isolate's heap,
    /// rooted by the caller until this returns.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn function_context_raw(
        &mut self,
        spec: &RawFunctionContext,
        out: Out<'_>,
        eval: &mut OnExitEval<'_>,
        body: impl FnOnce(&mut Cx, ContextId, Out<'_>) -> RResult<()>,
    ) -> RResult<()> {
        debug_assert!(spec.callflag & ctxt::FUNCTION != 0, "robigo: a function record without the FUNCTION bit");
        let (promargs, nargs, fill, filldata, lazy) = match spec.promargs {
            Ok(p) => (p, 0, None, 0, 0),
            Err((nargs, fill, data)) => (self.iso().nil, nargs, Some(fill), data, rflags::LAZY_ARGS),
        };
        let init = Init {
            nil: self.iso().nil,
            callflag: spec.callflag,
            rflags: lazy,
            call: spec.call,
            cloenv: spec.cloenv,
            sysparent: spec.sysparent,
            promargs,
            callfun: spec.callfun,
            nargs,
            fill,
            filldata,
        };
        self.run_function(init, out, eval, body)
    }

    #[inline(always)]
    fn run_function(
        &mut self,
        init: Init,
        out: Out<'_>,
        eval: &mut OnExitEval<'_>,
        body: impl FnOnce(&mut Cx, ContextId, Out<'_>) -> RResult<()>,
    ) -> RResult<()> {
        let eager = self.ctxs().eager_promargs.get() && init.rflags & rflags::LAZY_ARGS != 0;
        self.with_record(&init, |cx, c| {
            // (an error making promargs leaves the record like the body's)
            let filled = if eager { cx.materialize_promargs(c.id()) } else { Ok(()) };
            // R: eval.c:R_execClosure. The body writes the caller's slot;
            // its value is the record's returnValue ("make it available to
            // on.exit and implicitly protect"), and the slot is rewritten
            // below if the value that leaves is another.
            let slot = out.root();
            let mut res = filled.and_then(|()| body(cx, c.id(), out));
            if res.is_ok() {
                c.set_return_value(slot.bind(cx).sexp());
            }
            loop {
                match res {
                    Ok(()) => {
                        cx.ctxs().srcref.set(c.srcref());
                        match cx.end_function(c, eval) {
                            Ok(()) => break,
                            Err(e) => res = Err(e),
                        }
                    }
                    Err(e) => match cx.arrive(c, e) {
                        Arrival::Pass(e) => return Err(e),
                        Arrival::Landed(e) => {
                            let rec = cx.take_exit(e);
                            c.set_return_value(rec.value);
                            res = Ok(());
                        }
                        Arrival::Stop(e) => {
                            // an intermediate jump: returnValue is undefined
                            c.set_return_value(Sexp::NULL);
                            cx.ctxs().srcref.set(c.srcref());
                            res = Err(cx.end_at_stop(c, e, eval));
                        }
                    },
                }
            }
            let v = c.return_value();
            let v = if v.is_null() { cx.iso().nil } else { v };
            if slot.bind(cx).sexp() != v {
                let out = slot.reset(cx);
                // SAFETY: the record's returnValue is traced and nothing
                // collected since it was read.
                out.set(cx, unsafe { Ref::from_sexp(v, cx) });
            }
            Ok(())
        })
    }

    fn function_init(&self, spec: &FunctionContext<'_>) -> Init {
        let (promargs, nargs, fill, filldata, lazy) = match spec.promargs {
            Promargs::Filled(r) => (r.bind(self).sexp(), 0, None, 0, 0),
            Promargs::Lazy { nargs, fill, data } => (self.iso().nil, nargs, Some(fill), data, rflags::LAZY_ARGS),
        };
        Init {
            nil: self.iso().nil,
            callflag: spec.callflag,
            rflags: lazy,
            call: spec.call.bind(self).sexp(),
            cloenv: spec.cloenv.bind(self).sexp(),
            sysparent: spec.sysparent.bind(self).sexp(),
            promargs,
            callfun: spec.callfun.bind(self).sexp(),
            nargs,
            fill,
            filldata,
        }
    }

    /// `endcontext` on the normal path (the body returned, or a jump to
    /// the record landed): the `on.exit` code, with `R_ExitContext` set
    /// and the return value's link count raised while it runs. An exit out
    /// of the code is returned (it reaches the record again: the rest of
    /// the code still runs, or a `return` lands here).
    fn end_function(&mut self, c: Rec, eval: &mut OnExitEval<'_>) -> RResult<()> {
        let nil = self.iso().nil;
        // R: context.c:endcontext
        let hs = self.unwind_handler_stack(c.handlerstack());
        self.ctxs().handler_stack.set(hs);
        self.ctxs().restart_stack.set(c.restartstack());
        debug_assert!(c.jumptarget().is_null() || c.conexit() == nil);
        if c.cloenv() != nil && c.conexit() != nil {
            let st = self.ctxs();
            let s = c.conexit();
            let savevis = st.visible.get();
            let savecontext = st.exit_context.get();
            st.exit_context.set(c.ptr());
            c.set_conexit(nil); // prevent recursion
            c.set_jumptarget(null_mut()); // in case on.exit expr calls return()
            // (R_FixupExitingHandlerResult is S1's: its token does not
            // exist before S1 makes handler results.)
            let retval = c.return_value();
            if !retval.is_null() {
                // SAFETY: the record's returnValue is a live object.
                unsafe { retval.incref() };
            }
            self.run_conexit(c, s, eval)?;
            // (re-read: a moving collection may have moved it)
            let retval = c.return_value();
            if !retval.is_null() {
                // SAFETY: as above.
                unsafe { retval.decref() };
            }
            let st = self.ctxs();
            st.exit_context.set(savecontext);
            st.visible.set(savevis);
        }
        let st = self.ctxs();
        if st.exit_context.get() == c.ptr() {
            st.exit_context.set(null_mut());
        }
        Ok(())
    }

    /// `endcontext` at an intermediate stop (`jumptarget` set): the
    /// `on.exit` code runs with the exit parked in the record, then the
    /// exit resumes (a new leg). Returns the exit that leaves the record:
    /// the parked one, or one the code raised, which supersedes it.
    fn end_at_stop(&mut self, c: Rec, e: RError, eval: &mut OnExitEval<'_>) -> RError {
        let nil = self.iso().nil;
        // R: context.c:endcontext
        let hs = self.unwind_handler_stack(c.handlerstack());
        self.ctxs().handler_stack.set(hs);
        self.ctxs().restart_stack.set(c.restartstack());
        self.park(c, e);
        if c.cloenv() != nil && c.conexit() != nil {
            let st = self.ctxs();
            let s = c.conexit();
            let savevis = st.visible.get();
            let savecontext = st.exit_context.get();
            st.exit_context.set(c.ptr());
            c.set_conexit(nil);
            c.set_jumptarget(null_mut());
            // R_FixupExitingHandlerResult(saveretval): the parked exit's
            // value is GNU R's R_ReturnedValue
            let v = c.parked().map_or(nil, |r| r.value);
            if v != nil && !v.is_null() {
                let fix = self.iso().hooks.get().exit_value;
                self.scope(|cx, sc| {
                    let v = sc.root(cx, cx.raw_ref(v));
                    fix(cx, v);
                });
            }
            if let Err(e2) = self.run_conexit(c, s, eval) {
                // the code left by its own exit: the parked one is lost
                // (R_ExitContext and R_Visible stay as the jump left them)
                c.set_parked(None);
                return e2;
            }
            let st = self.ctxs();
            st.exit_context.set(savecontext);
            st.visible.set(savevis);
        }
        let st = self.ctxs();
        if st.exit_context.get() == c.ptr() {
            st.exit_context.set(null_mut());
        }
        // continue jumping: GNU R's R_jumpctxt(jumptarget, ...)
        self.unpark(c)
    }

    /// Evaluate the `on.exit` list `s` of `c` in `c`'s environment, one
    /// expression at a time, keeping the rest in `conexit` (so an exit out
    /// of one leaves the rest to run when it reaches the record).
    fn run_conexit(&mut self, c: Rec, s: Sexp, eval: &mut OnExitEval<'_>) -> RResult<()> {
        let nil = self.iso().nil;
        let mut s = s;
        while s != nil {
            s = self.scope(|cx, sc| {
                let cell = sc.root(cx, cx.raw_ref(s));
                // SAFETY: `s` is a live pairlist cell (traced through the
                // record's conexit until this iteration replaces it, and
                // rooted from here on).
                let (car, cdr) = unsafe { (s.car(), s.cdr()) };
                c.set_conexit(cdr);
                let expr = sc.root(cx, cx.raw_ref(car));
                let env = sc.root(cx, cx.raw_ref(c.cloenv()));
                eval(cx, expr, env)?;
                // (the rest, through the root: a moving collection may have
                // moved the cell)
                // SAFETY: the rooted cell is live.
                Ok(unsafe { cell.bind(cx).sexp().cdr() })
            })?;
        }
        Ok(())
    }

    /// A `Ref` for an object the caller knows to be live and reachable for
    /// this `&Cx` interval (a traced record field, a root's object).
    #[inline(always)]
    pub(crate) fn raw_ref(&self, x: Sexp) -> Ref<'_> {
        // SAFETY: the caller's knowledge, as the doc says: every use passes
        // a field of a live record or of an object reached through a root,
        // read in this interval.
        unsafe { Ref::from_sexp(x, self) }
    }

    // ---- records in C frames ------------------------------------------------

    /// GNU R's `begincontext(cptr, flags, call, env, sysp, promargs,
    /// callfun)` for a record in a C frame (C compiled against robigo's
    /// `Defn.h`; the export wrapper of `begincontext` calls this). The
    /// record becomes the innermost. An exit that passes it runs its `cend`
    /// before the jump destroys the frame; [`Cx::end_foreign`] ends it
    /// otherwise.
    ///
    /// # Safety
    /// `rec` is writable memory for an `RCNTXT` in a frame that stays
    /// until [`Cx::end_foreign`] or a jump past it; the objects are live
    /// objects of this isolate, rooted by the caller for the record's life
    /// (GNU R's callers protect them too). C code that sets the record's
    /// `cjmpbuf` with `SETJMP` and may be jumped to is not supported: only
    /// native entries land jumps.
    pub unsafe fn begin_foreign(
        &mut self,
        rec: NonNull<Context>,
        callflag: i32,
        call: Sexp,
        cloenv: Sexp,
        sysparent: Sexp,
        promargs: Sexp,
        callfun: Sexp,
    ) {
        let mut init = Init::plain(self, callflag, 0);
        init.call = call;
        init.cloenv = cloenv;
        init.sysparent = sysparent;
        init.promargs = promargs;
        init.callfun = callfun;
        self.begin(Rec(rec), &init);
    }

    /// GNU R's `endcontext(cptr)` for a record [`Cx::begin_foreign`]
    /// pushed, which must be the innermost. Records in C frames carry no
    /// `on.exit` code (only R code adds it, to function records the
    /// evaluator pushes); one that does panics.
    ///
    /// # Safety
    /// `rec` was pushed by [`Cx::begin_foreign`] and is still on the chain.
    pub unsafe fn end_foreign(&mut self, rec: NonNull<Context>) {
        let c = Rec(rec);
        assert!(self.ctxs().global.get() == c.ptr(), "robigo: endcontext of a context that is not the innermost");
        let nil = self.iso().nil;
        assert!(
            c.cloenv() == nil || c.conexit() == nil,
            "robigo: on.exit code in a context C pushed is not implemented yet"
        );
        // R: context.c:endcontext
        let hs = self.unwind_handler_stack(c.handlerstack());
        let st = self.ctxs();
        st.handler_stack.set(hs);
        st.restart_stack.set(c.restartstack());
        if st.exit_context.get() == c.ptr() {
            st.exit_context.set(null_mut());
        }
        st.global.set(c.nextcontext());
    }

    // ---- jumps -------------------------------------------------------------

    /// Record a jump to record `target` with GNU R's `mask`, carrying
    /// `value` (`R_ReturnedValue`): the exit `R_jumpctxt(target, mask,
    /// value)` makes, whose legs run as the `Err` travels the chain. A
    /// `CTXT_RESTART` mask records an [`crate::ExitKind::Restart`], others
    /// a [`crate::ExitKind::Jump`].
    pub fn jump_to(&mut self, target: ContextId, mask: i32, value: Option<Root<'_>>) -> RError {
        let kind = if mask & ctxt::RESTART != 0 { crate::ExitKind::Restart } else { crate::ExitKind::Jump };
        let info = crate::ExitInfo { kind, mask, target: target.addr() };
        self.exit(crate::Exit { info, value, condition: None, call: None })
    }

    /// GNU R's `findcontext(mask, env, val)`: `break`/`next` (a `LOOP`
    /// mask) to the innermost loop record of `env`, or `return` (a
    /// `FUNCTION` or `BROWSER` mask) to the innermost record of that type
    /// of `env`, not looking past a top-level record; GNU R's error
    /// otherwise.
    pub fn find_context(&mut self, mask: i32, env: Root<'_>, value: Option<Root<'_>>) -> RError {
        // R: context.c:findcontext
        let found = {
            let env = env.bind(self);
            let mut c = Some(self.global_context());
            let mut found = None;
            while let Some(r) = c {
                if r.callflag() == ctxt::TOPLEVEL {
                    break;
                }
                let hit =
                    if mask & ctxt::LOOP != 0 { r.callflag() & ctxt::LOOP != 0 } else { r.callflag() & mask != 0 };
                if hit && r.cloenv() == env {
                    found = Some(r.id());
                    break;
                }
                c = r.next();
            }
            found
        };
        match found {
            Some(t) => self.jump_to(t, mask, value),
            None if mask & ctxt::LOOP != 0 => self.error("no loop for break/next, jumping to top level"),
            None => self.error("no function to return from, jumping to top level"),
        }
    }

    /// GNU R's `R_JumpToContext(target, mask, val)`: a jump to `target`
    /// if it is on the chain before a top-level record (clearing
    /// `R_ExitContext` if it is passed on the way); GNU R's error
    /// otherwise.
    pub fn jump_to_context(&mut self, target: ContextId, mask: i32, value: Option<Root<'_>>) -> RError {
        // R: context.c:R_JumpToContext
        let st = self.ctxs();
        let mut c = Some(self.global_context());
        while let Some(r) = c {
            if r.callflag() == ctxt::TOPLEVEL {
                break;
            }
            if r.id() == target {
                return self.jump_to(target, mask, value);
            }
            if st.exit_context.get() == r.rec().ptr() {
                st.exit_context.set(null_mut());
            }
            c = r.next();
        }
        self.error("target context is not on the stack")
    }

    // ---- other records ---------------------------------------------------

    /// Run `body` in a record of type `callflag` (a loop, a browser or
    /// restart record, ...; not a function record, which needs
    /// [`Cx::function_context`]) with `call`, `cloenv` and `sysparent` as
    /// given. An exit that targets the record lands: it is resolved, its
    /// value written to `value` (if given), and its mask reported as
    /// [`Left::Landed`]. Other exits pass ([`Left::Exit`]).
    pub fn with_context<T>(
        &mut self,
        callflag: i32,
        call: Root<'_>,
        cloenv: Root<'_>,
        sysparent: Root<'_>,
        value: Option<Out<'_>>,
        body: impl FnOnce(&mut Cx, ContextId) -> RResult<T>,
    ) -> Result<T, Left> {
        debug_assert!(callflag & ctxt::FUNCTION == 0 && callflag != ctxt::UNWIND);
        let mut init = Init::plain(self, callflag, 0);
        init.call = call.bind(self).sexp();
        init.cloenv = cloenv.bind(self).sexp();
        init.sysparent = sysparent.bind(self).sexp();
        self.with_record(&init, |cx, c| {
            let r = body(cx, c.id());
            let r = match r {
                Ok(t) => Ok(t),
                Err(e) => match cx.arrive(c, e) {
                    Arrival::Pass(e) | Arrival::Stop(e) => Err(Left::Exit(e)),
                    Arrival::Landed(e) => {
                        let rec = cx.take_exit(e);
                        if let Some(out) = value {
                            out.set(cx, cx.raw_ref(rec.value));
                        }
                        Err(Left::Landed(rec.info))
                    }
                },
            };
            // R: context.c:endcontext (no on.exit code: not a function)
            let hs = cx.unwind_handler_stack(c.handlerstack());
            cx.ctxs().handler_stack.set(hs);
            cx.ctxs().restart_stack.set(c.restartstack());
            r
        })
    }

    /// The live record `id` for this `&Cx` interval, or None if it is no
    /// longer on the chain.
    pub fn context(&self, id: ContextId) -> Option<CtxRef<'_>> {
        self.ctxs().chain().find(|r| r.id() == id).map(|rec| CtxRef { rec, cx: self })
    }

    /// `R_GlobalContext` as R code sees it: the innermost record that is
    /// not hidden.
    pub fn global_context(&self) -> CtxRef<'_> {
        let rec = self.ctxs().chain().find(|r| !r.hidden()).expect("robigo: the context chain has no base");
        CtxRef { rec, cx: self }
    }

    /// `R_ToplevelContext`.
    pub fn toplevel_context(&self) -> CtxRef<'_> {
        // SAFETY: R_ToplevelContext is always a live record of the chain.
        let rec = Rec(unsafe { NonNull::new_unchecked(self.ctxs().toplevel.get()) });
        CtxRef { rec, cx: self }
    }

    /// The session's base record (`R_Toplevel`, `R_SessionContext`).
    pub fn session_context(&self) -> CtxRef<'_> {
        CtxRef { rec: self.ctxs().base(), cx: self }
    }

    /// `R_ExitContext`: the record whose `on.exit` code runs.
    pub fn exit_context(&self) -> Option<CtxRef<'_>> {
        let p = NonNull::new(self.ctxs().exit_context.get())?;
        Some(CtxRef { rec: Rec(p), cx: self })
    }

    /// How many records are on the chain, hidden and base included.
    pub fn context_depth(&self) -> usize {
        self.ctxs().chain().count()
    }

    /// Make the lazy `promargs` of record `id` (if it is lazy; C1's filler
    /// writes it). Panics if `id` is not on the chain.
    pub fn materialize_promargs(&mut self, id: ContextId) -> RResult<()> {
        let rec = self.context(id).expect("robigo: a context that is not on the chain").rec();
        if rec.rflags() & rflags::LAZY_ARGS == 0 {
            return Ok(());
        }
        let (fill, data) = rec.fill();
        let fill = fill.expect("robigo: lazy promargs without a filler");
        self.scope(|cx, s| {
            let out = s.out(cx);
            let r = out.root();
            fill(cx, id, data, out)?;
            // (the record is still live: its owner is below this frame)
            rec.set_promargs(r.bind(cx).sexp());
            rec.set_rflags(rec.rflags() & !rflags::LAZY_ARGS);
            Ok(())
        })
    }

    /// Set the type of the live record `id` (`cptr->callflag = ...`): S3
    /// dispatch marks the generic's record `CTXT_GENERIC` while a method
    /// runs, and back (objects.c:dispatchMethod, do_nextmethod). The type
    /// keeps its `FUNCTION` bit: a function record stays one. Panics if
    /// `id` is not on the chain or the bit would change.
    pub fn set_context_callflag(&mut self, id: ContextId, callflag: i32) {
        let rec = self.context(id).expect("robigo: a context that is not on the chain").rec();
        assert!(
            (rec.callflag() & ctxt::FUNCTION) == (callflag & ctxt::FUNCTION),
            "robigo: a record's FUNCTION bit cannot change"
        );
        rec.set_callflag(callflag);
    }

    /// Set the `sysparent` of the live record `id` (`NextMethod` makes its
    /// own record's the generic's caller, objects.c:do_nextmethod). Panics
    /// if `id` is not on the chain.
    pub fn set_context_sysparent(&mut self, id: ContextId, env: Root<'_>) {
        let rec = self.context(id).expect("robigo: a context that is not on the chain").rec();
        let e = env.bind(self).sexp();
        self.iso().check_owns(e);
        rec.set_sysparent(e);
    }

    /// Whether records fill `promargs` when they are pushed (set when
    /// compiled C that reads the field as a field is loaded).
    pub fn set_eager_promargs(&mut self, on: bool) {
        self.ctxs().eager_promargs.set(on);
    }

    /// `R_Srcref`.
    pub fn srcref(&self) -> Ref<'_> {
        self.raw_ref(self.ctxs().srcref.get())
    }

    pub fn set_srcref(&self, v: Ref<'_>) {
        self.iso().check_owns(v.sexp());
        self.ctxs().srcref.set(v.sexp());
    }

    /// `R_HandlerStack` and `R_RestartStack` (S1 builds their entries).
    pub fn handler_stack(&self) -> Ref<'_> {
        self.raw_ref(self.ctxs().handler_stack.get())
    }

    pub fn set_handler_stack(&mut self, v: Root<'_>) {
        let x = v.bind(self).sexp();
        self.ctxs().handler_stack.set(x);
    }

    pub fn restart_stack(&self) -> Ref<'_> {
        self.raw_ref(self.ctxs().restart_stack.get())
    }

    pub fn set_restart_stack(&mut self, v: Root<'_>) {
        let x = v.bind(self).sexp();
        self.ctxs().restart_stack.set(x);
    }
}
