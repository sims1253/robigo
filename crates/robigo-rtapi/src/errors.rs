//! errors.c and printutils.c for C code: the entries `api.c`'s variadic
//! `Rf_error`, `Rf_errorcall`, `Rf_warning`, `Rf_warningcall`, `Rprintf`
//! and `REprintf` hand their formatted text to, and those of its accessors'
//! error paths. Each error entry exits: its body never returns `Ok`.

use crate::entry;
use robigo_rtcore::types::*;
use robigo_rtcore::{Cx, RError, Ref, Root, Sexp};
use std::ffi::{CStr, c_char, c_int};

/// A C string from C, as text (GNU R's messages are bytes in the native
/// encoding, UTF-8 under robigo's locales).
pub(crate) fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: C's contract: a NUL-terminated string.
    String::from_utf8_lossy(unsafe { CStr::from_ptr(p) }.to_bytes()).into_owned()
}

/// GNU R's `R_typeToChar(x)`: "S4" or "object" for an `OBJSXP`, else the
/// type's name.
// R: util.c:R_typeToChar
pub(crate) fn r_type_to_char(x: Ref<'_>) -> &'static str {
    if x.sexptype() == OBJSXP {
        if x.is_s4() { "S4" } else { "object" }
    } else {
        robigo_rtbuiltins::util::type2char(x.sexptype())
    }
}

/// `errorcall(call, msg)`'s exit: R_NilValue is no call.
pub(crate) fn errorcall(cx: &mut Cx, call: Root<'_>, msg: &str) -> RError {
    if call.bind(cx).sexptype() == NILSXP { cx.error_call(None, msg) } else { cx.error_call(Some(call), msg) }
}

/// "robigo: '<what>' is not implemented yet" (AGENTS rule 6), as `error()`.
pub(crate) fn not_yet(cx: &mut Cx, what: &str) -> RError {
    cx.error(&format!("robigo: '{what}' is not implemented yet"))
}

entry! {
    /// GNU R's `error()` of a message `Rf_error` formatted: `errorcall`
    /// against the current context's call (the one below a foreign call's
    /// `CTXT_BUILTIN` record).
    // R: errors.c:error
    [1] fn robigo_rtapi_error(msg: *const c_char) -> () => |cx, _s| {
        Err(cx.error(&text(msg)))
    }

    /// GNU R's `errorcall(call, ...)` of a message `Rf_errorcall` formatted.
    // R: errors.c:errorcall
    [2] fn robigo_rtapi_errorcall(call: Sexp, msg: *const c_char) -> () => |cx, _s| {
        Err(errorcall(cx, call, &text(msg)))
    }

    /// An accessor applied to the wrong type: memory.c's "%s() can only be
    /// applied to a '%s', not a '%s'".
    // R: memory.c:INTEGER (and its siblings)
    [3] fn robigo_rtapi_type_error(fun: *const c_char, what: *const c_char, x: Sexp) -> () => |cx, _s| {
        let msg = format!("{}() can only be applied to a '{}', not a '{}'", text(fun), text(what), r_type_to_char(x.bind(cx)));
        Err(cx.error(&msg))
    }

    /// An element index out of range: memory.c's "attempt access index
    /// %lld/%lld in %s".
    // R: memory.c:VECTOR_ELT
    [3] fn robigo_rtapi_index_error(fun: *const c_char, i: isize, n: isize) -> () => |cx, _s| {
        Err(cx.error(&format!("attempt access index {i}/{n} in {}", text(fun))))
    }

    /// memory.c's `CHKEXTPTRSXP`.
    // R: memory.c:CHKEXTPTRSXP
    [2] fn robigo_rtapi_extptr_error(fun: *const c_char, x: Sexp) -> () => |cx, _s| {
        let t = robigo_rtbuiltins::util::type2char(x.bind(cx).sexptype());
        Err(cx.error(&format!("{}: argument of type {t} is not an external pointer", text(fun))))
    }

    /// `PROTECT` past `R_PPStackSize`: GNU R's `R_signal_protect_error`
    /// signals a preallocated `protectStackOverflowError` condition inside
    /// the red zone, which needs the condition system's classes.
    // R: memory.c:R_signal_protect_error
    [0] fn robigo_rtapi_protect_error() -> () => |cx, _s| {
        Err(not_yet(cx, "R_signal_protect_error (a protect stack overflow's condition)"))
    }

    /// `UNPROTECT(n)` of more than the stack holds.
    // R: memory.c:R_signal_unprotect_error
    [0] fn robigo_rtapi_unprotect_error() -> () => |cx, _s| {
        let n = cx.pp_top();
        let what = if n == 1 { "item" } else { "items" };
        Err(cx.error(&format!("unprotect(): only {n} protected {what}")))
    }

    /// `R_Reprotect` of an index not on the stack.
    // R: memory.c:R_signal_reprotect_error
    [1] fn robigo_rtapi_reprotect_error(i: c_int) -> () => |cx, _s| {
        let n = cx.pp_top();
        let what = if n == 1 { "item" } else { "items" };
        Err(cx.error(&format!("R_Reprotect: only {n} protected {what}, can't reprotect index {i}")))
    }

    /// A case of an accessor robigo does not have yet (AGENTS rule 6).
    [1] fn robigo_rtapi_not_yet(what: *const c_char) -> () => |cx, _s| {
        Err(not_yet(cx, &text(what)))
    }

    /// GNU R's `warning()` of a message `Rf_warning` formatted: against the
    /// current context's call, as `error()`.
    // R: errors.c:warning
    [1] fn robigo_rtapi_warning(msg: *const c_char) -> () => |cx, _s| {
        cx.warning(&text(msg))
    }

    /// GNU R's `warningcall(call, ...)`.
    // R: errors.c:warningcall
    [2] fn robigo_rtapi_warningcall(call: Sexp, msg: *const c_char) -> () => |cx, _s| {
        let m = text(msg);
        if call.bind(cx).sexptype() == NILSXP { cx.warning_call(None, &m) } else { cx.warning_call(Some(call), &m) }
    }
}

/// `R_WarnLength` (`options(warning.length)`, default 1000), which bounds
/// `error()`'s and `warning()`'s messages. robigo's `options()` refuses
/// `warning.length` until the condition system reads it, so it is GNU R's
/// default.
#[unsafe(no_mangle)]
pub extern "C" fn robigo_rtapi_warn_length() -> c_int {
    1000
}

/// `R_WriteConsole` (or its error stream) of `n` bytes `Rprintf` formatted:
/// the isolate's console. Cannot collect or exit.
///
/// # Safety
/// `buf` holds `n` bytes; called from C code a native entry runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_rtapi_print(to_stderr: c_int, buf: *const c_char, n: usize) {
    // SAFETY: the caller's contract.
    let b = unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), n) };
    // SAFETY: called from C code of the thread's isolate.
    unsafe {
        robigo_rtcore::enter_leaf(|cx| if to_stderr != 0 { cx.reprintf(b) } else { cx.rprintf(b) });
    }
}

/// The heap's barrier store of `v` into the traced field `slot` of `x`
/// (GNU R's `FIX_REFCNT` and `CHECK_OLD_TO_NEW`, then the write): what
/// `api.c`'s setters call. Cannot collect or exit.
///
/// # Safety
/// `x` is a live object, `slot` one of its traced pointer fields, `v` null
/// or a live object of the same heap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_rtapi_store(x: Sexp, slot: *mut Sexp, v: Sexp) {
    // SAFETY: the caller's contract.
    unsafe { x.store(slot, v) }
}
