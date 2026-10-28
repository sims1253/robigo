//! The host runtime's callbacks (`docs/core.md` §6). Before G3 the
//! evaluator, environments and conditions live in robigo-runtime; it
//! hands `libR.so` this table in [`robigo_capi_init`].
//!
//! Every callback that can fail returns a *flow token*: 0 for success,
//! otherwise an identifier of a non-local exit (an R error, `return`, a
//! restart, an interrupt) the host keeps until the jump reaches the
//! context that hands it back ([`crate::dotcode`]).

use crate::state::st;
use robigo_core::Sexp;
use std::ffi::{c_char, c_int};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Host {
    /// Evaluate `expr` in `env`.
    pub eval: unsafe extern "C" fn(expr: Sexp, env: Sexp, out: *mut Sexp) -> usize,
    /// Write to the console: stream 0 is stdout (Rprintf), 1 stderr (REprintf).
    pub write: unsafe extern "C" fn(stream: c_int, buf: *const u8, len: usize),
    /// Signal a warning; `call` is NULL for the context call (R's
    /// `warning()`), else the call to report (`warningcall`). `immediate`
    /// for `warningcall_immediate`.
    pub warning: unsafe extern "C" fn(call: Sexp, msg: *const c_char, immediate: c_int) -> usize,
    /// Everything else, by name (environments, printing, coercion, ...):
    /// `args`/`n` in, `*out` the result. Unknown names fail with the
    /// porting contract's "not implemented" error.
    pub op: unsafe extern "C" fn(name: *const c_char, args: *const Sexp, n: c_int, out: *mut Sexp) -> usize,
    /// A flow token for an R error with this message (`call` NULL: the
    /// context call), for errors raised inside libR (`Rf_error`).
    pub error: unsafe extern "C" fn(call: Sexp, msg: *const c_char) -> usize,
    /// Print the error behind `token` the way the top level does (for
    /// `R_ToplevelExec`, `R_tryEval`) and forget it.
    pub report: unsafe extern "C" fn(token: usize),
    /// Forget a flow token that will not be continued.
    pub discard: unsafe extern "C" fn(token: usize),
    /// The host's part of a context record (its condition handler and
    /// restart stacks, which signalling pops as it searches): save into
    /// two words when a context is entered, restore them when a jump
    /// reaches it (context.c R_restore_globals), drop them otherwise.
    pub ctx_save: unsafe extern "C" fn(out: *mut usize),
    pub ctx_restore: unsafe extern "C" fn(saved: *const usize),
    pub ctx_drop: unsafe extern "C" fn(saved: *const usize),
}

/// Install the host's callbacks and create the heap. `globals` receives
/// the host's environments as proxies created with
/// [`robigo_host_proxy`]: global, base, empty, base namespace, namespace
/// registry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_capi_init(host: *const Host) {
    crate::state::init_state();
    st().host = Some(*host);
}

/// Set one of the API's environment globals: 0 R_GlobalEnv, 1 R_BaseEnv,
/// 2 R_EmptyEnv, 3 R_BaseNamespace, 4 R_NamespaceRegistry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_set_global(which: c_int, v: Sexp) {
    use crate::state::*;
    match which {
        0 => R_GlobalEnv = v,
        1 => R_BaseEnv = v,
        2 => R_EmptyEnv = v,
        3 => R_BaseNamespace = v,
        4 => R_NamespaceRegistry = v,
        _ => {}
    }
}

pub fn host() -> &'static Host {
    match &st().host {
        Some(h) => h,
        None => {
            eprintln!("robigo: libR.so used before robigo_capi_init");
            std::process::abort()
        }
    }
}

/// Call a host operation; jumps on failure.
pub unsafe fn op(name: &'static str, args: &[Sexp]) -> Sexp {
    debug_assert!(name.ends_with('\0'));
    let mut out = Sexp::NULL;
    let t = (host().op)(name.as_ptr() as *const c_char, args.as_ptr(), args.len() as c_int, &mut out);
    if t != 0 {
        crate::errors::jump_host(t);
    }
    out
}

/// Call a host operation; returns the flow token instead of jumping.
pub unsafe fn try_op(name: &'static str, args: &[Sexp], out: &mut Sexp) -> usize {
    (host().op)(name.as_ptr() as *const c_char, args.as_ptr(), args.len() as c_int, out)
}

/// Write to the console.
pub fn write(stream: c_int, s: &[u8]) {
    unsafe { (host().write)(stream, s.as_ptr(), s.len()) }
}

/// A proxy for a host object (HOST flag): `release(handle)` runs once the
/// proxy is dead.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_host_proxy(t: c_int, handle: usize, release: Option<robigo_core::ReleaseFn>) -> Sexp {
    crate::state::core().alloc_host(t as u8, handle, release)
}

/// The handle of a HOST proxy, or 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_host_handle(x: Sexp) -> usize {
    if x.has_flag(robigo_core::flags::HOST) { robigo_core::Core::host_handle(x) } else { 0 }
}

/// A vector over a buffer owned by the host: `release(owner)` runs once
/// it is dead.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_extdata(
    t: c_int,
    data: *mut u8,
    len: isize,
    owner: usize,
    release: Option<robigo_core::ReleaseFn>,
) -> Sexp {
    crate::state::core().alloc_extdata(t as u8, data, len as usize, owner, release)
}

/// The owner word of an EXTDATA vector, or 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_extdata_owner(x: Sexp) -> usize {
    if x.has_flag(robigo_core::flags::EXTDATA) { robigo_core::Core::extdata_owner(x) } else { 0 }
}

/// Header flags and gp bits (for the host's conversions).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_flags(x: Sexp) -> c_int {
    x.flags() as c_int
}

/// Collect now; returns the pause in nanoseconds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_gc() -> u64 {
    let t = crate::state::core().collect();
    crate::util::safe_point();
    t
}
