//! Native entries (docs/g3.md §6.5, `docs/core.md` §5): C code called
//! from a context record whose `cjmpbuf` is the jump record of the heap's
//! trampoline (`robigo_heap::unwind`), and the jump back out of C.
//!
//! - Rust calls C only through these: the record is pushed in the Rust
//!   frame, flagged `LANDING`, the capability is suspended
//!   ([`Cx::call_out`]), and the trampoline sets the record and calls the
//!   code with no Rust frame in between. It also opens a C segment of the
//!   conservative scan for the callee (H2's protocol).
//! - C leaves only through an export wrapper: the Rust part of the export
//!   returns an `Err`, [`Cx::prepare_jump`] runs what GNU R's `R_jumpctxt`
//!   runs before its `LONGJMP` (the leg's `cend`s, innermost first, from the
//!   record C runs in), and once the export's Rust frames are gone the
//!   wrapper calls [`Landing::jump`]: the innermost native entry's
//!   trampoline returns 1, and its Rust owner sees the exit as an `Err`
//!   reaching its record.
//!
//! The records GNU R has (`R_ToplevelExec`'s `CTXT_TOPLEVEL`,
//! `R_UnwindProtect`'s `CTXT_UNWIND`, `R_ExecWithCleanup`'s `CTXT_CCODE`, a
//! foreign call's `CTXT_BUILTIN`) are these records; the native entries GNU
//! R makes no context for (package ALTREP methods, `cend`s) push a hidden
//! one.

#![allow(unsafe_code)]

use crate::context::{Arrival, ContextId, Init, Rec, ctxt, rflags};
use crate::exit::{ExitInfo, ExitRecord, RError, RResult, kind_from_u8};
use crate::handle::Ref;
use crate::isolate::Cx;
use crate::root::{Out, Root};
use robigo_heap::unwind::{JmpBuf, Ret6, robigo_heap_try_call6, robigo_heap_try_call65};
use robigo_heap::{NeedGc, Sexp, types};
use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

/// What a six-argument callee returned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Native6 {
    Word(usize),
    Double(f64),
    Complex([f64; 2]),
}

/// Where an export wrapper jumps: the innermost native entry's record,
/// which [`Cx::prepare_jump`] made `R_GlobalContext`. The exit stays
/// pending; the trampoline's owner picks it up.
#[must_use = "a Landing stands for a pending exit: jump to it"]
#[derive(Debug)]
pub struct Landing {
    jb: NonNull<JmpBuf>,
    rec: ContextId,
}

impl Landing {
    /// The record jumped to.
    pub fn context(&self) -> ContextId {
        self.rec
    }

    /// `LONGJMP` to the record: its native entry returns 1.
    ///
    /// # Safety
    /// The isolate's capability was obtained by the export wrapper this is
    /// called from ([`crate::enter_raw`]) and has ended, and every frame
    /// between here and the native entry is a C frame or the wrapper's own,
    /// with nothing to drop (`robigo_heap::unwind::jump`'s contract). The
    /// export reopened its C segment first.
    pub unsafe fn jump(self) -> ! {
        // SAFETY: the record's native entry is still running (it is on the
        // chain, which prepare_jump checked), and the caller's contract.
        unsafe { robigo_heap::unwind::jump(self.jb.as_ptr()) }
    }
}

impl Cx {
    /// Run `enter` (a heap trampoline call: jump record, segment slot,
    /// `.Call` plain switch) with the capability suspended; true if an exit
    /// jumped to `c`'s record, which is then the innermost.
    fn enter_native(&mut self, c: Rec, enter: impl FnOnce(*mut JmpBuf, *mut usize, c_int) -> c_int) -> bool {
        // SAFETY: the slot and the switch are read through a reference that
        // ends with this statement; the slot's address is stable (the core
        // is boxed), and the trampoline writes it only while no Rust code of
        // ours runs (or through an export, as the heap's protocol says).
        let (scan, plain) = unsafe {
            let core = self.iso().core();
            (core.scan_top_slot(), core.call65_plain() as c_int)
        };
        let jb = c.jmpbuf();
        let jumped = self.call_out(|_| enter(jb, scan, plain)) != 0;
        debug_assert!(
            !jumped || self.ctxs().global.get() == c.ptr(),
            "robigo: a jump landed at a record that is not the innermost"
        );
        jumped
    }

    /// `endcontext` for a record without `on.exit` code: the handler and
    /// restart stacks (`R_UnwindHandlerStack`).
    fn end_plain(&mut self, c: Rec) {
        if c.hidden() {
            return; // GNU R has no context here
        }
        let hs = self.unwind_handler_stack(c.handlerstack());
        self.ctxs().handler_stack.set(hs);
        self.ctxs().restart_stack.set(c.restartstack());
    }

    /// A package routine called by `.Call`/`.External`: `f(args[0..65])`
    /// from a `CTXT_BUILTIN` record for `call` (GNU R's `eval` makes one for
    /// `PP_FOREIGN` builtins, `begincontext(&cntxt, CTXT_BUILTIN, e,
    /// R_BaseEnv, R_BaseEnv, ...)`). Returns the routine's result word.
    ///
    /// # Safety
    /// As `robigo_heap_try_call65` (`f`'s shape, the 65 slots), and every
    /// object in `args` is rooted by the caller. The result is a raw word:
    /// the caller roots it before anything may collect.
    pub unsafe fn call_native65(
        &mut self,
        call: Root<'_>,
        base_env: Root<'_>,
        f: *const c_void,
        args: &mut [*mut c_void; 65],
    ) -> RResult<*mut c_void> {
        let mut init = Init::plain(self, ctxt::BUILTIN, rflags::LANDING);
        init.call = call.bind(self).sexp();
        init.cloenv = base_env.bind(self).sexp();
        init.sysparent = init.cloenv;
        self.with_record(&init, |cx, c| {
            let mut out = std::ptr::null_mut();
            let a = args.as_mut_ptr();
            let o = &raw mut out;
            // SAFETY: the caller's contract; the record (and its jump
            // record) outlives the call.
            let jumped =
                cx.enter_native(c, |jb, scan, plain| unsafe { robigo_heap_try_call65(jb, f, a, o, scan, plain) });
            if !jumped {
                cx.end_plain(c);
                return Ok(out);
            }
            Err(cx.leave_native(c))
        })
    }

    /// An exit jumped to a native entry's record `c` that is not its
    /// target (`.Call`, ALTREP and `cend` records are never targets): it
    /// passes.
    fn leave_native(&mut self, c: Rec) -> RError {
        match self.arrive(c, RError::new()) {
            Arrival::Pass(e) | Arrival::Stop(e) | Arrival::Landed(e) => e,
        }
    }

    /// A callback of up to six word arguments (a package ALTREP method, a
    /// C finalizer, ...) as a native entry from a hidden record (GNU R
    /// calls these directly: R code sees no context for them).
    ///
    /// # Safety
    /// As `robigo_heap_try_call6`: `f` takes at most six integer or pointer
    /// arguments and returns as `ret` says; objects among `a` are rooted
    /// by the caller, and a returned object word is rooted by the caller
    /// before anything may collect.
    pub unsafe fn call_native6(&mut self, f: *const c_void, a: [usize; 6], ret: Ret6) -> RResult<Native6> {
        let init = Init::plain(self, ctxt::CCODE, rflags::LANDING | rflags::HIDDEN);
        self.with_record(&init, |cx, c| {
            let mut buf = [0u64; 2];
            let o = buf.as_mut_ptr().cast::<c_void>();
            // SAFETY: the caller's contract; `buf` holds any of the three
            // return kinds.
            let jumped =
                cx.enter_native(c, |jb, scan, _| unsafe { robigo_heap_try_call6(jb, f, a.as_ptr(), ret, o, scan) });
            if jumped {
                return Err(cx.leave_native(c));
            }
            Ok(match ret {
                Ret6::Word => Native6::Word(buf[0] as usize),
                Ret6::Double => Native6::Double(f64::from_bits(buf[0])),
                Ret6::Complex => Native6::Complex([f64::from_bits(buf[0]), f64::from_bits(buf[1])]),
            })
        })
    }

    /// [`Cx::call_native6`] for a word or nothing.
    ///
    /// # Safety
    /// As [`Cx::call_native6`].
    pub(crate) unsafe fn native6_hidden(&mut self, f: *const c_void, a: [usize; 6]) -> RResult<usize> {
        // SAFETY: the caller's contract.
        match unsafe { self.call_native6(f, a, Ret6::Word) }? {
            Native6::Word(w) => Ok(w),
            _ => unreachable!(),
        }
    }

    /// A top-level record (`R_ToplevelExec`'s): `R_HandlerStack` and
    /// `R_RestartStack` empty while `body` runs in it, the record
    /// `R_ToplevelContext`, and everything restored after. Ok(true): `body`
    /// finished; Ok(false): an exit to the record landed there (an error
    /// with no handler outside the record: GNU R reported it before the
    /// jump). An exit to a record further out passes (GNU R's `longjmp`
    /// would leave `R_ToplevelExec`'s frame too).
    fn toplevel_with(
        &mut self,
        global_env: Root<'_>,
        base_env: Root<'_>,
        landing: bool,
        body: impl FnOnce(&mut Cx, Rec) -> Result<(), Option<RError>>,
    ) -> RResult<bool> {
        let nil = self.iso().nil;
        // R: context.c:R_ToplevelExec
        let st = self.ctxs();
        let old_h = st.handler_stack.get();
        let old_r = st.restart_stack.get();
        let oldvis = st.visible.get();
        let save_top = st.toplevel.get();
        st.handler_stack.set(nil);
        st.restart_stack.set(nil);
        let mut init = Init::plain(self, ctxt::TOPLEVEL, if landing { rflags::LANDING } else { 0 });
        init.cloenv = global_env.bind(self).sexp();
        init.sysparent = base_env.bind(self).sexp();
        // the saved stacks stay roots (R_ToplevelExec PROTECTs them) in a
        // scope around the record
        self.scope(|cx, s| {
            let old_h = s.root(cx, cx.raw_ref(old_h));
            let old_r = s.root(cx, cx.raw_ref(old_r));
            let r = cx.with_record(&init, |cx, c| {
                let _top = RestoreToplevel { iso: cx.iso_ptr(), top: save_top };
                cx.ctxs().toplevel.set(c.ptr());
                let r = match body(cx, c) {
                    Ok(()) => Ok(true),
                    Err(e) => {
                        let e = match e {
                            Some(e) => e,
                            None => RError::new(), // jumped to this record
                        };
                        match cx.arrive(c, e) {
                            Arrival::Landed(e) => {
                                cx.take_exit(e);
                                Ok(false)
                            }
                            Arrival::Pass(e) | Arrival::Stop(e) => return Err(e),
                        }
                    }
                };
                cx.end_plain(c);
                r
            });
            let st = cx.ctxs();
            st.handler_stack.set(old_h.bind(cx).sexp());
            st.restart_stack.set(old_r.bind(cx).sexp());
            st.visible.set(oldvis);
            r
        })
    }

    /// `R_ToplevelExec(fun, data)`: `fun(data)` as a native entry from a
    /// top-level record; Ok(true) if it returned, Ok(false) if an exit to
    /// the record landed (an error, which GNU R reported before jumping).
    /// `global_env` and `base_env` are the record's `cloenv` and
    /// `sysparent` (`R_GlobalEnv`, `R_BaseEnv`).
    ///
    /// # Safety
    /// `fun` is a C function of one pointer argument returning nothing,
    /// callable with `data`.
    pub unsafe fn toplevel_exec(
        &mut self,
        global_env: Root<'_>,
        base_env: Root<'_>,
        fun: unsafe extern "C" fn(*mut c_void),
        data: *mut c_void,
    ) -> RResult<bool> {
        self.toplevel_with(global_env, base_env, true, |cx, c| {
            let a = [data as usize, 0, 0, 0, 0, 0];
            // SAFETY: the caller's contract.
            let jumped = cx.enter_native(c, |jb, scan, _| unsafe {
                robigo_heap_try_call6(jb, fun as *const c_void, a.as_ptr(), Ret6::Word, std::ptr::null_mut(), scan)
            });
            if jumped { Err(None) } else { Ok(()) }
        })
    }

    /// A top-level record around Rust code (the finalizer runner, the
    /// REPL's evaluation of one expression): [`Cx::toplevel_exec`]'s
    /// semantics for `body`.
    pub fn toplevel(
        &mut self,
        global_env: Root<'_>,
        base_env: Root<'_>,
        body: impl FnOnce(&mut Cx) -> RResult<()>,
    ) -> RResult<bool> {
        self.toplevel_with(global_env, base_env, false, |cx, _| body(cx).map_err(Some))
    }

    /// A new `R_UnwindProtect` continuation (`R_MakeUnwindCont`): robigo's
    /// shape is the pairlist `(value, condition, call, info)`, `info` a raw
    /// vector of the exit's kind, mask and target; it owns an exit's record
    /// from the jump to `R_UnwindProtect`'s record until
    /// [`Cx::continue_unwind`].
    pub fn make_unwind_cont<'g>(&'g self) -> Result<Ref<'g>, NeedGc> {
        let nil = self.nil();
        let info = self.alloc_copy(&[0u8; CONT_INFO])?.finish().into();
        let c3 = self.cons(nil, info)?;
        let c2 = self.cons(nil, c3)?;
        self.cons(nil, c2)
    }

    /// `R_UnwindProtect(fun, data, ..., cont)` up to its cleanup: `fun(data)`
    /// as a native entry from a `CTXT_UNWIND` record. Ok(false): `fun`
    /// returned, and its result is the car of `cont`. Ok(true): an exit
    /// stopped at the record, which every exit does (`first_jump_target`);
    /// `cont` now owns its record (value, condition, call, kind, mask,
    /// target), and the caller calls `cleanfun(cleandata, TRUE)` and then
    /// [`Cx::continue_unwind`]. Err: an exit passed (it was not one this
    /// record stops: never for a well-formed chain).
    ///
    /// The cleanup is the caller's (the export's) to call, from its own
    /// frame, as GNU R does: cpp11 and Rcpp leave it by `longjmp` and resume
    /// the exit later with `R_ContinueUnwind`.
    ///
    /// # Safety
    /// `fun` is a C function `SEXP fun(void *)` callable with `data`;
    /// `cont` is a continuation from [`Cx::make_unwind_cont`].
    pub unsafe fn unwind_protect(
        &mut self,
        global_env: Root<'_>,
        base_env: Root<'_>,
        fun: *const c_void,
        data: *mut c_void,
        cont: Root<'_>,
    ) -> RResult<bool> {
        let cont_x = cont.bind(self).sexp();
        assert!(is_cont(cont_x), "robigo: R_UnwindProtect's continuation was not made by R_MakeUnwindCont");
        let mut init = Init::plain(self, ctxt::UNWIND, rflags::LANDING);
        init.cloenv = global_env.bind(self).sexp();
        init.sysparent = base_env.bind(self).sexp();
        self.with_record(&init, |cx, c| {
            let a = [data as usize, 0, 0, 0, 0, 0];
            let mut out = 0usize;
            let o = (&raw mut out).cast::<c_void>();
            // SAFETY: the caller's contract.
            let jumped = cx.enter_native(c, |jb, scan, _| unsafe {
                robigo_heap_try_call6(jb, fun, a.as_ptr(), Ret6::Word, o, scan)
            });
            // R: context.c:R_UnwindProtect
            let cont_x = cont.bind(cx).sexp();
            let r = if !jumped {
                // (rooted through `cont` from here; nothing collected since
                // fun returned)
                // SAFETY: `cont` is a live continuation (checked above) and
                // `out` the object fun returned.
                unsafe { cont_x.set_car(Sexp(out as *mut u8)) };
                Ok(false)
            } else {
                match cx.arrive(c, RError::new()) {
                    Arrival::Stop(e) | Arrival::Landed(e) => {
                        let rec = cx.take_exit(e);
                        // the record's jumptarget goes to the continuation
                        c.set_jumptarget(std::ptr::null_mut());
                        // SAFETY: as above; the record's objects were roots
                        // until the pop and nothing collected since.
                        unsafe { store_cont(cont_x, &rec) };
                        Ok(true)
                    }
                    Arrival::Pass(e) => return Err(e),
                }
            };
            cx.end_plain(c);
            r
        })
    }

    /// `R_ContinueUnwind(cont)`: the exit `cont` owns is pending again, its
    /// next leg starting from the innermost record (`R_jumpctxt`). An
    /// export wrapper then [`Cx::prepare_jump`]s and jumps.
    pub fn continue_unwind(&mut self, cont: Root<'_>) -> RError {
        let x = cont.bind(self).sexp();
        if !is_cont(x) {
            return self
                .error_call_sexp(self.iso().nil, "robigo: R_ContinueUnwind: not a continuation from R_MakeUnwindCont");
        }
        // SAFETY: a live continuation (checked), its cells and info vector
        // live with it.
        let rec = unsafe { load_cont(x) };
        self.iso().exit_push(rec);
        RError::new()
    }

    /// `R_ExecWithCleanup(fun, data, cleanfun, cleandata)`: `fun(data)` as
    /// a native entry from a `CTXT_CCODE` record whose `cend` is `cleanfun`;
    /// then `cleanfun(cleandata)` (still in the record) and the record ends.
    /// `fun`'s result is written to `out`. An exit out of `fun` runs the
    /// `cend` in its leg before the jump; one out of `cleanfun` runs it
    /// again, as GNU R does.
    ///
    /// # Safety
    /// `fun` is `SEXP fun(void *)` callable with `data`, returning a live
    /// object; `cleanfun` is `void cleanfun(void *)` callable with
    /// `cleandata`.
    pub unsafe fn exec_with_cleanup(
        &mut self,
        base_env: Root<'_>,
        fun: *const c_void,
        data: *mut c_void,
        cleanfun: unsafe extern "C" fn(*mut c_void),
        cleandata: *mut c_void,
        out: Out<'_>,
    ) -> RResult<()> {
        let mut init = Init::plain(self, ctxt::CCODE, rflags::LANDING);
        init.cloenv = base_env.bind(self).sexp();
        init.sysparent = init.cloenv;
        self.with_record(&init, |cx, c| {
            c.set_cend(Some(cleanfun));
            c.set_cenddata(cleandata);
            let a = [data as usize, 0, 0, 0, 0, 0];
            let mut res = 0usize;
            let o = (&raw mut res).cast::<c_void>();
            // SAFETY: the caller's contract.
            let jumped = cx.enter_native(c, |jb, scan, _| unsafe {
                robigo_heap_try_call6(jb, fun, a.as_ptr(), Ret6::Word, o, scan)
            });
            if jumped {
                return Err(cx.leave_native(c));
            }
            // R: context.c:R_ExecWithCleanup (PROTECT(result = fun(data)))
            // SAFETY: the caller's contract: fun returned a live object, and
            // nothing collected since.
            out.set(cx, unsafe { Ref::from_sexp(Sexp(res as *mut u8), cx) });
            let a = [cleandata as usize, 0, 0, 0, 0, 0];
            // SAFETY: the caller's contract for cleanfun.
            if let Err(e) = unsafe { cx.native6_hidden(cleanfun as *const c_void, a) } {
                // the record is the innermost again; its cend runs in the
                // leg (GNU R's R_run_onexits)
                return Err(match cx.arrive(c, e) {
                    Arrival::Pass(e) | Arrival::Stop(e) | Arrival::Landed(e) => e,
                });
            }
            cx.end_plain(c);
            Ok(())
        })
    }

    /// What an export wrapper does with an exit before it jumps (GNU R's
    /// `R_jumpctxt` up to its `LONGJMP`): start the exit's leg from the
    /// record C code runs in (the `cend`s above the leg's stop run now,
    /// innermost first, while every frame is still there), then make the
    /// innermost native entry's record `R_GlobalContext`. The records above
    /// it were C's, and are gone with the jump. Call [`Landing::jump`]
    /// once the export's capability has ended.
    ///
    /// Aborts if no native entry is on the chain: C code that runs outside
    /// one cannot be left.
    pub fn prepare_jump(&mut self, e: RError) -> Landing {
        let e = self.start_leg(e);
        let RError { .. } = e; // (the record stays pending; the trampoline's owner resumes it)
        let st = self.ctxs();
        let Some(l) = st.chain().find(|r| r.landing()) else {
            eprintln!("robigo: an exit from C code that no native entry called");
            std::process::abort();
        };
        st.global.set(l.ptr());
        // SAFETY: a record's jump buffer is a field of a live record.
        Landing { jb: unsafe { NonNull::new_unchecked(l.jmpbuf()) }, rec: l.id() }
    }
}

/// Restores `R_ToplevelContext` when a top-level record ends, on every
/// path.
struct RestoreToplevel {
    iso: NonNull<crate::isolate::Isolate>,
    top: *mut crate::context::Context,
}

impl Drop for RestoreToplevel {
    fn drop(&mut self) {
        // SAFETY: made inside `Cx::toplevel_with`, whose borrow keeps the
        // isolate alive.
        unsafe { self.iso.as_ref() }.contexts().toplevel.set(self.top);
    }
}

/// Bytes of a continuation's info vector: kind (u32), mask (i32), target.
const CONT_INFO: usize = 16;

/// Whether `x` has a continuation's shape.
fn is_cont(x: Sexp) -> bool {
    // SAFETY: `x` is a live object from a root; the cells and vector read
    // are checked to exist before they are read.
    unsafe {
        let ok = |y: Sexp| y.typ() == types::LISTSXP;
        if !ok(x) || !ok(x.cdr()) || !ok(x.cdr().cdr()) {
            return false;
        }
        let info = x.cdr().cdr().cdr();
        info.typ() == types::RAWSXP && !info.is_altrep() && info.std_len() as usize == CONT_INFO
    }
}

/// Write an exit's record into the continuation `x`.
///
/// # Safety
/// `x` is a live continuation and the record's objects are live.
unsafe fn store_cont(x: Sexp, rec: &ExitRecord) {
    // SAFETY: the caller's contract; `is_cont`'s shape.
    unsafe {
        x.set_car(rec.value);
        x.cdr().set_car(rec.condition);
        x.cdr().cdr().set_car(rec.call);
        let p = x.cdr().cdr().cdr().std_data();
        p.cast::<u32>().write_unaligned(rec.info.kind as u32);
        p.add(4).cast::<i32>().write_unaligned(rec.info.mask);
        p.add(8).cast::<u64>().write_unaligned(rec.info.target as u64);
    }
}

/// The exit's record a continuation `x` owns, its leg not started.
///
/// # Safety
/// `x` is a live continuation.
unsafe fn load_cont(x: Sexp) -> ExitRecord {
    // SAFETY: the caller's contract; `is_cont`'s shape.
    unsafe {
        let p = x.cdr().cdr().cdr().std_data();
        ExitRecord {
            info: ExitInfo {
                kind: kind_from_u8(p.cast::<u32>().read_unaligned() as u8),
                mask: p.add(4).cast::<i32>().read_unaligned(),
                target: p.add(8).cast::<u64>().read_unaligned() as usize,
            },
            value: x.car(),
            condition: x.cdr().car(),
            call: x.cdr().cdr().car(),
            stop: 0,
        }
    }
}
