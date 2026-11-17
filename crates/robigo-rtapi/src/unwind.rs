//! context.c's exports (docs/g3.md §6.5): `R_ToplevelExec`,
//! `R_UnwindProtect`'s protected part (the cleanup is `api.c`'s, called
//! from C), `R_ContinueUnwind`, `R_MakeUnwindCont` and
//! `R_ExecWithCleanup`, over robigo-rtcore's native entries.

use crate::entry;
use crate::memory::alloc;
use robigo_rtcore::{RResult, Sexp};
use std::ffi::{c_int, c_void};

/// `void (*)(void *)`: `R_ToplevelExec`'s fun, a cleanup.
type VoidFn = unsafe extern "C" fn(*mut c_void) -> ();

entry! {
    /// `R_ToplevelExec(fun, data)`: TRUE if `fun` returned, FALSE if an
    /// error ended it (reported before the jump, as GNU R's default
    /// handler does).
    // R: context.c:R_ToplevelExec
    [2] fn R_ToplevelExec(fun: VoidFn, data: *mut c_void) -> c_int => |cx, s| {
        let global = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        // SAFETY: C's contract for R_ToplevelExec's fun.
        let ok = unsafe { cx.toplevel_exec(global, base, fun, data) }?;
        Ok(ok as c_int)
    }

    /// `R_UnwindProtect`'s context and protected call (context.c, from
    /// `begincontext` to `endcontext`): 0 and `*result` set if `fun`
    /// returned; 1 if an exit stopped at the context, which `cont` now
    /// owns. `api.c`'s `R_UnwindProtect` then calls the cleanup and
    /// `R_ContinueUnwind`.
    // R: context.c:R_UnwindProtect
    [4] fn robigo_rtapi_unwind_protect(fun: *const c_void, data: *mut c_void, cont: Sexp, result: *mut Sexp) -> c_int => |cx, s| {
        let global = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        // SAFETY: C's contract for R_UnwindProtect's fun; `cont` is a
        // continuation of R_MakeUnwindCont (checked: otherwise it panics).
        let jumped = unsafe { cx.unwind_protect(global, base, fun, data, cont) }?;
        if !jumped {
            // SAFETY: C's contract: a SEXP to write; the result is the car
            // of the continuation, which C keeps.
            unsafe { *result = cont.bind(cx).sexp().car() };
        }
        Ok(jumped as c_int)
    }

    /// `R_ContinueUnwind(cont)`: the exit `cont` owns continues.
    // R: context.c:R_ContinueUnwind
    [1] fn R_ContinueUnwind(cont: Sexp) -> () => |cx, _s| {
        Err(cx.continue_unwind(cont))
    }

    /// `R_CheckUserInterrupt()`: a pending user interrupt taken (the
    /// condition's handlers, then the jump to the top level), unless
    /// interrupts are suspended.
    // R: errors.c:R_CheckUserInterrupt
    [0] fn R_CheckUserInterrupt() -> () => |cx, _s| {
        cx.check_user_interrupt()
    }

    /// `R_MakeUnwindCont()`.
    // R: context.c:R_MakeUnwindCont
    [0] fn R_MakeUnwindCont() -> Sexp => |cx, s| {
        alloc(cx, s, |cx| cx.make_unwind_cont())
    }

    /// `R_ExecWithCleanup(fun, data, cleanfun, cleandata)`.
    // R: context.c:R_ExecWithCleanup
    [4] fn R_ExecWithCleanup(fun: *const c_void, data: *mut c_void, cleanfun: VoidFn, cleandata: *mut c_void) -> Sexp => |cx, s| {
        let base = s.root(cx, cx.base_env());
        let o = s.out(cx);
        let r = o.root();
        // SAFETY: C's contract for R_ExecWithCleanup's functions.
        unsafe { cx.exec_with_cleanup(base, fun, data, cleanfun, cleandata, o) }?;
        let v: RResult<Sexp> = Ok(r.bind(cx).sexp());
        v
    }
}

/// `begincontext(cptr, flags, syscall, env, sysp, promargs, callfun)` on a
/// record in a C frame (C compiled against robigo's `Defn.h`): written and
/// linked as the innermost. It allocates nothing and cannot fail, so it is
/// a leaf: no machine entry.
///
/// # Safety
/// C's contract: `cptr` is an `RCNTXT` of its frame, which it ends with
/// `endcontext` (or an exit leaves); the objects are live and protected.
// R: context.c:begincontext
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_begincontext(
    cptr: *mut c_void,
    flags: c_int,
    syscall: Sexp,
    env: Sexp,
    sysp: Sexp,
    promargs: Sexp,
    callfun: Sexp,
) {
    let Some(rec) = std::ptr::NonNull::new(cptr.cast()) else {
        eprintln!("robigo: begincontext of a null context");
        std::process::abort()
    };
    // SAFETY: called from C code a native entry of the thread's isolate
    // runs; begin_foreign writes the record and links it, collecting
    // nothing; C's contract for the record and the objects.
    unsafe {
        robigo_rtcore::enter_leaf_mut(|cx| cx.begin_foreign(rec, flags, syscall, env, sysp, promargs, callfun))
    }
}

entry! {
    /// `endcontext(cptr)` on the innermost record, one C began: the handler
    /// and restart stacks restored, the record unlinked.
    // R: context.c:endcontext
    [1] fn Rf_endcontext(cptr: *mut c_void) -> () => |cx, _s| {
        let Some(rec) = std::ptr::NonNull::new(cptr.cast()) else {
            return Err(cx.error("robigo: endcontext of a null context is not implemented yet"));
        };
        // SAFETY: C's contract: a record it began and has not ended.
        unsafe { cx.end_foreign(rec) }
    }
}
