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
