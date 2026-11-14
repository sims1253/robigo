//! The C API's per-isolate state (docs/g3.md §11, L6; §3 "What a collection
//! marks", item 1): the globals GNU R keeps for C code (`R_NilValue`,
//! `R_GlobalEnv`, the symbol shortcuts, the protect stack `R_PPStack` with
//! `R_PPStackTop`, `R_alloc`'s stack `R_VStack`) in one block per isolate,
//! [`CGlobals`], the layout robigo's `include/robigo_capi.h` declares.
//!
//! C reaches the block through the C API's one thread-local, `robigo_cg`
//! (`capi.c`, initial-exec): every native entry makes it the block of the
//! isolate whose code it calls and restores the previous one when the code
//! returns or is left ([`Cx::call_out`]), so several isolates may run on
//! one thread, one at a time, and on several threads at once.
//!
//! The protect stack and `R_VStack` are precise roots; a context saves
//! their tops when it is pushed and an exit restores them at its stop, as
//! GNU R's `begincontext` and `R_restore_globals` do (`context.rs`). The
//! objects the block names directly are this isolate's constants, fixed
//! environments and symbols, marked as seen by C when the isolate is made:
//! `ROBIGO_GC_MOVE=1` never moves them, so C's copies stay right.

#![allow(unsafe_code)]

use crate::isolate::{Cx, Isolate};
use robigo_heap::{Sexp, Trace, Visitor, types};
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int};

/// The symbol shortcuts' count (`ROBIGO_CAPI_SYMBOLS` in `robigo_capi.h`).
pub const SYMBOLS: usize = 39;

/// GNU R's default protect stack size (`R_PPSSIZE`, Defn.h) and its red
/// zone (`PP_REDZONE_SIZE`, memory.c): room the stack keeps past its size
/// for the error report of an overflow.
pub const PPSTACK_SIZE: usize = 50_000;
pub const PP_REDZONE: usize = 1000;

/// The per-isolate C globals, as `robigo_capi.h` declares them. C reads and
/// writes the cells while the isolate's capability is suspended; Rust code
/// reads them through `Cx`.
#[repr(C)]
pub struct CGlobals {
    pub(crate) nil: Sexp,
    pub(crate) global_env: Sexp,
    pub(crate) empty_env: Sexp,
    pub(crate) base_env: Sexp,
    pub(crate) base_namespace: Sexp,
    pub(crate) unbound: Sexp,
    pub(crate) missing_arg: Sexp,
    pub(crate) na_string: Sexp,
    pub(crate) blank_string: Sexp,
    pub(crate) blank_scalar_string: Sexp,
    pub(crate) symbols: [Sexp; SYMBOLS],
    pub(crate) ppstack: *mut Cell<Sexp>,
    pub(crate) pptop: Cell<c_int>,
    pub(crate) ppsize: c_int,
    pub(crate) vstack: Cell<Sexp>,
    pub(crate) gc_move: Cell<c_int>,
    /// the isolate (`*mut Isolate`, opaque to C)
    pub(crate) isolate: *mut std::ffi::c_void,
}

unsafe extern "C" {
    static robigo_cglobals_layout: [usize; 0];
    static robigo_cglobals_layout_len: usize;
    static robigo_cglobals_symbols: [*const c_char; SYMBOLS];
    static robigo_cglobals_nsymbols: usize;
    fn robigo_rtcore_cg() -> *mut CGlobals;
    fn robigo_rtcore_set_cg(g: *mut CGlobals);
}

/// The layout `capi.c` exports: size, alignment, each field's offset.
pub fn c_layout() -> Vec<usize> {
    // SAFETY: `capi.c` defines the table and its length as constants.
    unsafe {
        let n = robigo_cglobals_layout_len;
        std::slice::from_raw_parts(robigo_cglobals_layout.as_ptr(), n).to_vec()
    }
}

/// Rust's view of the same table.
fn rust_layout() -> Vec<usize> {
    use std::mem::offset_of;
    let mut v = vec![size_of::<CGlobals>(), align_of::<CGlobals>()];
    v.extend([
        offset_of!(CGlobals, nil),
        offset_of!(CGlobals, global_env),
        offset_of!(CGlobals, empty_env),
        offset_of!(CGlobals, base_env),
        offset_of!(CGlobals, base_namespace),
        offset_of!(CGlobals, unbound),
        offset_of!(CGlobals, missing_arg),
        offset_of!(CGlobals, na_string),
        offset_of!(CGlobals, blank_string),
        offset_of!(CGlobals, blank_scalar_string),
    ]);
    v.extend((0..SYMBOLS).map(|i| offset_of!(CGlobals, symbols) + i * size_of::<Sexp>()));
    v.extend([
        offset_of!(CGlobals, ppstack),
        offset_of!(CGlobals, pptop),
        offset_of!(CGlobals, ppsize),
        offset_of!(CGlobals, vstack),
        offset_of!(CGlobals, gc_move),
        offset_of!(CGlobals, isolate),
    ]);
    v
}

/// Panics unless `robigo_capi.h`'s layout is [`CGlobals`]'s.
pub fn check_layout() {
    // SAFETY: a constant of `capi.c`.
    assert_eq!(unsafe { robigo_cglobals_nsymbols }, SYMBOLS, "robigo: robigo_capi.h lists another number of symbols");
    assert_eq!(c_layout(), rust_layout(), "robigo: robigo_capi.h and capi.rs disagree on the C globals' layout");
}

/// The names of the symbol shortcuts, in [`CGlobals`]'s order.
pub fn symbol_names() -> Vec<&'static [u8]> {
    // SAFETY: `capi.c`'s table of string literals.
    unsafe { robigo_cglobals_symbols.iter().map(|&p| CStr::from_ptr(p).to_bytes()).collect() }
}

/// The block of the isolate whose native code runs on this thread (null:
/// none), the C API's thread-local.
#[inline]
pub fn current() -> *mut CGlobals {
    // SAFETY: reads a thread-local word.
    unsafe { robigo_rtcore_cg() }
}

/// Make `g` the thread's block; returns the previous one.
#[inline]
pub(crate) fn set_current(g: *mut CGlobals) -> *mut CGlobals {
    // SAFETY: reads and writes a thread-local word.
    unsafe {
        let old = robigo_rtcore_cg();
        if old != g {
            robigo_rtcore_set_cg(g);
        }
        old
    }
}

/// Restore the thread's block (a native entry's end).
#[inline]
pub(crate) fn restore_current(old: *mut CGlobals, now: *mut CGlobals) {
    if old != now {
        // SAFETY: writes a thread-local word.
        unsafe { robigo_rtcore_set_cg(old) }
    }
}

/// The isolate's C API state: the block and the protect stack's storage.
pub(crate) struct CApi {
    pub(crate) g: CGlobals,
    /// `R_PPStack`: `PPSTACK_SIZE` slots and the red zone
    stack: Box<[Cell<Sexp>]>,
    /// `R_FalseValue`, `R_TrueValue`, `R_LogicalNAValue`
    pub(crate) logicals: [Sexp; 3],
}

impl CApi {
    /// The block of a new isolate. `isolate` is filled in once the isolate
    /// has its address ([`CApi::set_isolate`]).
    pub(crate) fn new(consts: [Sexp; 10], symbols: [Sexp; SYMBOLS], logicals: [Sexp; 3]) -> CApi {
        let [nil, global_env, empty_env, base_env, base_namespace, unbound, missing_arg, na_string, blank_string, bss] =
            consts;
        let stack: Box<[Cell<Sexp>]> = (0..PPSTACK_SIZE + PP_REDZONE).map(|_| Cell::new(Sexp::NULL)).collect();
        let mut c = CApi {
            g: CGlobals {
                nil,
                global_env,
                empty_env,
                base_env,
                base_namespace,
                unbound,
                missing_arg,
                na_string,
                blank_string,
                blank_scalar_string: bss,
                symbols,
                ppstack: std::ptr::null_mut(),
                pptop: Cell::new(0),
                ppsize: PPSTACK_SIZE as c_int,
                vstack: Cell::new(nil),
                gc_move: Cell::new(0),
                isolate: std::ptr::null_mut(),
            },
            stack,
            logicals,
        };
        // the boxed slots do not move with the struct
        c.g.ppstack = c.stack.as_ptr().cast_mut();
        for x in consts.iter().chain(symbols.iter()).chain(logicals.iter()) {
            // SAFETY: live objects of a fresh heap; the bit is sticky header
            // state that only the debug mover reads.
            unsafe { x.mark_seen_by_c() };
        }
        c
    }

    pub(crate) fn set_isolate(&mut self, iso: *mut Isolate) {
        self.g.isolate = iso.cast();
    }
}

/// The block's objects, the live protect-stack slots and `R_VStack`.
impl Trace for CApi {
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        let g = &mut self.g;
        for x in [
            &mut g.nil,
            &mut g.global_env,
            &mut g.empty_env,
            &mut g.base_env,
            &mut g.base_namespace,
            &mut g.unbound,
            &mut g.missing_arg,
            &mut g.na_string,
            &mut g.blank_string,
            &mut g.blank_scalar_string,
        ] {
            v.edge(x);
        }
        for s in g.symbols.iter_mut() {
            v.edge(s);
        }
        let top = (g.pptop.get().max(0) as usize).min(self.stack.len());
        for c in &self.stack[..top] {
            let mut x = c.get();
            v.edge(&mut x);
            c.set(x);
        }
        let mut x = g.vstack.get();
        v.edge(&mut x);
        g.vstack.set(x);
        for x in self.logicals.iter_mut() {
            v.edge(x);
        }
    }
}

impl Cx {
    /// The isolate's C globals block (what `robigo_cg` names while its
    /// native code runs).
    #[inline(always)]
    pub(crate) fn cglobals(&self) -> &CGlobals {
        self.iso().capi_globals()
    }

    /// The block's address, for the C side.
    #[inline(always)]
    pub fn cglobals_ptr(&self) -> *mut CGlobals {
        (self.cglobals() as *const CGlobals).cast_mut()
    }

    /// `R_PPStackTop`.
    #[inline(always)]
    pub fn pp_top(&self) -> i32 {
        self.cglobals().pptop.get()
    }

    /// `R_PPStackSize`.
    #[inline(always)]
    pub fn pp_size(&self) -> i32 {
        self.cglobals().ppsize
    }

    /// Set `R_PPStackTop` (a native call's balance restored, an exit's
    /// stop). `top` is at most the current top or a top saved earlier.
    #[inline(always)]
    pub fn set_pp_top(&mut self, top: i32) {
        self.cglobals().pptop.set(top);
    }

    /// The protect stack's slot `i` (below the top), for tests and checks.
    pub fn pp_slot(&self, i: usize) -> Option<crate::Ref<'_>> {
        let g = self.cglobals();
        if i >= g.pptop.get().max(0) as usize {
            return None;
        }
        // SAFETY: a slot below the top holds a live object C protected
        // (a precise root), within the stack's storage.
        unsafe { Some(crate::Ref::from_sexp((*g.ppstack.add(i)).get(), self)) }
    }

    /// `R_VStack` (`vmaxget()`): R_alloc's newest block, or R_NilValue.
    #[inline(always)]
    pub fn vmax(&self) -> Sexp {
        self.cglobals().vstack.get()
    }

    /// `vmaxset(v)`: drop the `R_alloc` blocks made since `vmaxget()`
    /// returned `v`.
    ///
    /// # Safety
    /// `v` is R_NilValue or a block on the stack now (`vmaxget()` of a
    /// frame still live).
    #[inline(always)]
    pub unsafe fn set_vmax(&mut self, v: Sexp) {
        self.cglobals().vstack.set(v);
    }

    /// Push the block `x` onto `R_alloc`'s stack (linked through its
    /// attribute slot, as memory.c's `R_alloc` does).
    ///
    /// # Safety
    /// `x` is a fresh, unpublished vector of this heap with no attributes.
    pub unsafe fn push_vmax(&mut self, x: Sexp) {
        let g = self.cglobals();
        // SAFETY: the caller's contract: a fresh vector; the old top is a
        // live block or R_NilValue.
        unsafe { x.set_attrib(g.vstack.get()) };
        g.vstack.set(x);
    }

    /// The symbol shortcut `i` (in `robigo_capi.h`'s order).
    pub fn capi_symbol(&self, i: usize) -> crate::Ref<'_> {
        // SAFETY: an immortal symbol of this heap.
        unsafe { crate::Ref::from_sexp(self.cglobals().symbols[i], self) }
    }

    /// `R_BlankScalarString`.
    pub fn blank_scalar_string(&self) -> crate::Ref<'_> {
        // SAFETY: rooted by the block.
        unsafe { crate::Ref::from_sexp(self.cglobals().blank_scalar_string, self) }
    }

    /// Whether objects handed to C are marked as seen by C
    /// (`ROBIGO_GC_MOVE=1`).
    #[inline(always)]
    pub fn marks_seen_by_c(&self) -> bool {
        self.cglobals().gc_move.get() != 0
    }

    /// Mark `x` as handed to C if the moving debug collector is on: it then
    /// never moves (C may keep its address where no collection looks).
    #[inline(always)]
    pub fn hand_to_c(&self, x: Sexp) {
        if self.marks_seen_by_c() && !x.is_null() {
            // SAFETY: `x` is a live object (the callers pass objects from
            // roots, arguments and results); the bit is sticky header state.
            unsafe { x.mark_seen_by_c() }
        }
    }
}

/// The constants a block names: `R_NilValue`, the four fixed
/// environments, `R_UnboundValue`, `R_MissingArg`, `NA_STRING`, `""` and
/// `R_BlankScalarString` (a fresh `STRSXP` of `""`).
pub(crate) fn constants(core: &mut robigo_heap::Core, envs: &crate::env::EnvState) -> [Sexp; 10] {
    let nil = core.nil();
    let blank = core.blank_string();
    let bss = core.alloc_vector(types::STRSXP, 1).expect("robigo: no room for R_BlankScalarString");
    // SAFETY: fresh; its one element is a CHARSXP of this heap.
    unsafe { bss.set_elt(0, blank) };
    let (global, empty, base, base_ns) = envs.fixed();
    [nil, global, empty, base, base_ns, core.unbound(), core.missing_arg(), core.na_string(), blank, bss]
}

/// The symbol shortcuts of a new heap.
pub(crate) fn symbols(core: &mut robigo_heap::Core) -> [Sexp; SYMBOLS] {
    let names = symbol_names();
    std::array::from_fn(|i| core.install(names[i]).expect("robigo: no room for the C API's symbols"))
}

/// `R_FalseValue`, `R_TrueValue` and `R_LogicalNAValue` of a new heap
/// (names.c's `InitNames` makes them; `MARK_NOT_MUTABLE`: a saturated
/// count).
pub(crate) fn logicals(core: &mut robigo_heap::Core) -> [Sexp; 3] {
    std::array::from_fn(|i| {
        let x = core.alloc_vector(types::LGLSXP, 1).expect("robigo: no room for R_TrueValue");
        let v = [0, 1, robigo_heap::NA_LOGICAL][i];
        // SAFETY: fresh, a logical vector of one element.
        unsafe {
            *(x.std_data() as *mut i32) = v;
            x.set_refcnt(u16::MAX);
        }
        x
    })
}
