//! dotcode.c's calling side: the host invokes native routines through
//! these entry points, inside a context (`docs/core.md` §5). A jump out
//! of the routine (an R error, an interrupt, a non-local exit of R code
//! called back) comes back as the flow token.

use crate::state::*;
use robigo_core::Sexp;
use std::ffi::{c_int, c_void};

/// R limits `.Call` and `.C` to 65 arguments (dotcode.c MAX_ARGS).
pub const MAX_ARGS: usize = 65;

/// Protect-stack headroom below a native routine (see robigo_dotcall).
const PP_HEADROOM: c_int = 8;

/// What the bridge reports after a call: the protect stack depth before
/// and after (for dotcode.c's "stack imbalance" warning).
#[repr(C)]
pub struct CallInfo {
    pub pp_before: c_int,
    pub pp_after: c_int,
}

/// `.Call`: `fun(args...)`. Returns 0 and the result in `*out`, or the
/// flow token of a jump. The arguments are protected for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_dotcall(
    fun: *const c_void,
    args: *const Sexp,
    nargs: c_int,
    out: *mut Sexp,
    info: *mut CallInfo,
) -> usize {
    let n = nargs.clamp(0, MAX_ARGS as c_int) as usize;
    let mut a = [R_NilValue; MAX_ARGS];
    a[..n].copy_from_slice(std::slice::from_raw_parts(args, n));
    let base = R_PPStackTop;
    for &x in &a[..n] {
        crate::memory::Rf_protect(x);
    }
    // GNU R's evaluator always holds protections below a .Call (5 to 23
    // in measurements, 8 for a top-level print(.Call(...))): code that
    // unprotects more than it protected gets the "stack imbalance"
    // warning there, not an error. Keep that headroom above the
    // arguments, and report depths relative to 8.
    for _ in 0..PP_HEADROOM {
        crate::memory::Rf_protect(R_NilValue);
    }
    let before = R_PPStackTop;
    let r = crate::errors::try_call65(fun, &mut a);
    let after = R_PPStackTop;
    R_PPStackTop = base;
    if !info.is_null() {
        (*info).pp_before = PP_HEADROOM;
        (*info).pp_after = PP_HEADROOM + (after - before);
    }
    match r {
        Ok(v) => {
            *out = v;
            crate::util::safe_point();
            0
        }
        Err(t) => {
            crate::util::safe_point();
            t
        }
    }
}

/// `.External`: `fun(args)` with the argument pairlist (whose car is the
/// routine's name or symbol object, as dotcode.c passes it).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_dotexternal(
    fun: *const c_void,
    args: Sexp,
    out: *mut Sexp,
    info: *mut CallInfo,
) -> usize {
    robigo_dotcall(fun, &args, 1, out, info)
}

/// `.External2`: `fun(call, op, args, env)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_dotexternal2(
    fun: *const c_void,
    call: Sexp,
    op: Sexp,
    args: Sexp,
    env: Sexp,
    out: *mut Sexp,
    info: *mut CallInfo,
) -> usize {
    let a = [call, op, args, env];
    robigo_dotcall(fun, a.as_ptr(), 4, out, info)
}

/// `.C`/`.Fortran`: `fun(ptrs...)` over buffers the host prepared.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_dotc(fun: *const c_void, ptrs: *const *mut c_void, nargs: c_int) -> usize {
    let n = nargs.clamp(0, MAX_ARGS as c_int) as usize;
    let mut a = [Sexp::NULL; MAX_ARGS];
    for (k, p) in std::slice::from_raw_parts(ptrs, n).iter().enumerate() {
        a[k] = Sexp(*p as *mut u8);
    }
    let base = R_PPStackTop;
    let r = crate::errors::try_call65(fun, &mut a);
    R_PPStackTop = base;
    crate::util::safe_point();
    r.err().unwrap_or(0)
}

/// Run the finalizers queued by the last collection (at safe points: after
/// a native call returns, from `gc()`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_run_finalizers() {
    crate::util::safe_point()
}
