//! dotcode.c's calling side: the host invokes native routines through
//! these entry points, inside a context (`docs/core.md` §5). A jump out
//! of the routine (an R error, an interrupt, a non-local exit of R code
//! called back) comes back as the flow token.

use crate::state::*;
use robigo_core::Sexp;
use std::ffi::{c_int, c_void};

/// R limits `.Call` and `.C` to 65 arguments (dotcode.c MAX_ARGS).
pub const MAX_ARGS: usize = 65;

type Fn65<T> = unsafe extern "C" fn(
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
    T,
) -> Sexp;

/// Call `f` with `a` padded to 65 arguments. On the SysV ABIs a callee
/// ignores arguments it does not declare, so one call shape serves every
/// arity (dotcode.c switches over 65 casts instead).
unsafe fn call65<T: Copy>(f: *const c_void, a: &[T; MAX_ARGS]) -> Sexp {
    let f: Fn65<T> = std::mem::transmute(f);
    f(
        a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9], a[10], a[11], a[12], a[13], a[14], a[15], a[16],
        a[17], a[18], a[19], a[20], a[21], a[22], a[23], a[24], a[25], a[26], a[27], a[28], a[29], a[30], a[31], a[32],
        a[33], a[34], a[35], a[36], a[37], a[38], a[39], a[40], a[41], a[42], a[43], a[44], a[45], a[46], a[47], a[48],
        a[49], a[50], a[51], a[52], a[53], a[54], a[55], a[56], a[57], a[58], a[59], a[60], a[61], a[62], a[63], a[64],
    )
}

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
    let before = R_PPStackTop;
    let vmax = crate::memory::vmaxget();
    let ap = &a as *const [Sexp; MAX_ARGS];
    let o = out;
    let r = crate::errors::protect_run(|| *o = call65(fun, &*ap));
    let after = R_PPStackTop;
    R_PPStackTop = base;
    crate::memory::vmaxset(vmax);
    if !info.is_null() {
        (*info).pp_before = before;
        (*info).pp_after = after;
    }
    r.unwrap_or(0)
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
    let mut a = [std::ptr::null_mut::<c_void>(); MAX_ARGS];
    a[..n].copy_from_slice(std::slice::from_raw_parts(ptrs, n));
    let base = R_PPStackTop;
    let vmax = crate::memory::vmaxget();
    let ap = &a as *const [*mut c_void; MAX_ARGS];
    let r = crate::errors::protect_run(|| {
        call65(fun, &*ap);
    });
    R_PPStackTop = base;
    crate::memory::vmaxset(vmax);
    r.unwrap_or(0)
}

/// Run the finalizers queued by the last collection (at safe points: after
/// a native call returns, from `gc()`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_run_finalizers() {
    crate::util::run_finalizers()
}
