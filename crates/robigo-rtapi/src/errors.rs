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
///
/// # Safety
/// `p` is null or a NUL-terminated string that stays put during the call.
pub(crate) unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the caller's contract.
    String::from_utf8_lossy(unsafe { CStr::from_ptr(p) }.to_bytes()).into_owned()
}

/// GNU R's `R_typeToChar(x)` (one copy, robigo-rtbuiltins').
pub(crate) use robigo_rtbuiltins::util::type_to_char as r_type_to_char;

/// `errorcall(call, msg)`'s exit: R_NilValue is no call.
pub(crate) fn errorcall(cx: &mut Cx, call: Root<'_>, msg: &str) -> RError {
    if call.bind(cx).sexptype() == NILSXP { cx.error_call(None, msg) } else { cx.error_call(Some(call), msg) }
}

/// `SET_STRING_ELT`'s "Value of SET_STRING_ELT() must be a 'CHARSXP' not a
/// '%s'".
// R: memory.c:SET_STRING_ELT
pub(crate) fn charsxp_error(cx: &mut Cx, v: Root<'_>) -> RError {
    let t = r_type_to_char(v.bind(cx));
    cx.error(&format!("Value of SET_STRING_ELT() must be a 'CHARSXP' not a '{t}'"))
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
        // SAFETY: C's contract: NUL-terminated strings, kept for the call.
        let msg = unsafe { text(msg) };
        Err(cx.error(&msg))
    }

    /// GNU R's `errorcall(call, ...)` of a message `Rf_errorcall` formatted.
    // R: errors.c:errorcall
    [2] fn robigo_rtapi_errorcall(call: Sexp, msg: *const c_char) -> () => |cx, _s| {
        // SAFETY: C's contract: NUL-terminated strings, kept for the call.
        let msg = unsafe { text(msg) };
        Err(errorcall(cx, call, &msg))
    }

    /// An accessor applied to the wrong type: memory.c's "%s() can only be
    /// applied to a '%s', not a '%s'".
    // R: memory.c:INTEGER (and its siblings)
    [3] fn robigo_rtapi_type_error(fun: *const c_char, what: *const c_char, x: Sexp) -> () => |cx, _s| {
        // SAFETY: C's contract: NUL-terminated strings, kept for the call.
        let (fun, what) = unsafe { (text(fun), text(what)) };
        let msg = format!("{fun}() can only be applied to a '{what}', not a '{}'", r_type_to_char(x.bind(cx)));
        Err(cx.error(&msg))
    }

    /// `CHKVEC`'s error: `DATAPTR` and its kin of an object that is not a
    /// vector.
    // R: Rinlinedfuns.h:CHKVEC
    [1] fn robigo_rtapi_dataptr_type_error(x: Sexp) -> () => |cx, _s| {
        let msg = format!("cannot get data pointer of '{}' objects", r_type_to_char(x.bind(cx)));
        Err(cx.error(&msg))
    }

    /// An element index out of range: memory.c's "attempt access index
    /// %lld/%lld in %s".
    // R: memory.c:VECTOR_ELT
    [3] fn robigo_rtapi_index_error(fun: *const c_char, i: isize, n: isize) -> () => |cx, _s| {
        // SAFETY: C's contract: NUL-terminated strings, kept for the call.
        let fun = unsafe { text(fun) };
        Err(cx.error(&format!("attempt access index {i}/{n} in {fun}")))
    }

    /// A setter's index out of range: memory.c's "attempt to set index
    /// %lld/%lld in %s".
    // R: memory.c:SET_VECTOR_ELT
    [3] fn robigo_rtapi_set_index_error(fun: *const c_char, i: isize, n: isize) -> () => |cx, _s| {
        // SAFETY: C's contract: NUL-terminated strings, kept for the call.
        let fun = unsafe { text(fun) };
        Err(cx.error(&format!("attempt to set index {i}/{n} in {fun}")))
    }

    /// `SET_STRING_ELT` of a value that is not a CHARSXP.
    // R: memory.c:SET_STRING_ELT
    [1] fn robigo_rtapi_charsxp_error(v: Sexp) -> () => |cx, _s| {
        Err(charsxp_error(cx, v))
    }

    /// memory.c's `CHKEXTPTRSXP`.
    // R: memory.c:CHKEXTPTRSXP
    [2] fn robigo_rtapi_extptr_error(fun: *const c_char, x: Sexp) -> () => |cx, _s| {
        let t = robigo_rtbuiltins::util::sexptype2char(x.bind(cx).sexptype());
        // SAFETY: C's contract: NUL-terminated strings, kept for the call.
        let fun = unsafe { text(fun) };
        Err(cx.error(&format!("{fun}: argument of type {t} is not an external pointer")))
    }

    /// `PROTECT` past `R_PPStackSize`, from api.c's
    /// `R_signal_protect_error` (its context restores the size as the exit
    /// leaves): the `protectStackOverflowError` condition (NULL call). With
    /// the red zone still closed it is opened and the condition's calling
    /// handlers run; once it is open they are skipped (they might overflow
    /// again).
    // R: memory.c:R_signal_protect_error
    [0] fn robigo_rtapi_protect_error() -> () => |cx, _s| {
        let cond = if cx.pp_size() < cx.pp_real_size() {
            cx.set_pp_size(cx.pp_real_size());
            robigo_rtcore::PROT_SO
        } else {
            robigo_rtcore::ErrorCond { exit_only: true, ..robigo_rtcore::PROT_SO }
        };
        Err(cx.error_condition_call(None, &cond))
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
        // SAFETY: C's contract: NUL-terminated strings, kept for the call.
        let what = unsafe { text(what) };
        Err(not_yet(cx, &what))
    }

    /// GNU R's `warning()` of a message `Rf_warning` formatted: against the
    /// current context's call, as `error()`.
    // R: errors.c:warning
    [1] fn robigo_rtapi_warning(msg: *const c_char) -> () => |cx, _s| {
        // SAFETY: C's contract: NUL-terminated strings, kept for the call.
        let msg = unsafe { text(msg) };
        cx.warning(&msg)
    }

    /// GNU R's `warningcall(call, ...)`.
    // R: errors.c:warningcall
    [2] fn robigo_rtapi_warningcall(call: Sexp, msg: *const c_char) -> () => |cx, _s| {
        // SAFETY: C's contract: NUL-terminated strings, kept for the call.
        let m = unsafe { text(msg) };
        if call.bind(cx).sexptype() == NILSXP { cx.warning_call(None, &m) } else { cx.warning_call(Some(call), &m) }
    }
}

/// `R_WarnLength` (`options(warning.length)`, the isolate's), which bounds
/// `error()`'s and `warning()`'s messages. Cannot collect or exit.
///
/// # Safety
/// Called from C code a native entry runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_rtapi_warn_length() -> c_int {
    // SAFETY: called from C code of the thread's isolate.
    unsafe { robigo_rtcore::enter_leaf(|cx| cx.report_options().warn_length.get() as c_int) }
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

/// C's first writable `DATAPTR` of a frozen vector of objects (api.c): the
/// object is logged as a root, once, so that C's raw writes into its
/// elements are traced (robigo-heap's `freeze.rs`).
///
/// # Safety
/// `x` is a live object of the thread's isolate.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_rtapi_remember(x: Sexp) {
    // SAFETY: called from C code of the thread's isolate, with `x` a live
    // object of its heap (the caller's contract).
    unsafe { robigo_rtcore::enter_leaf(|cx| cx.remember_raw(x)) };
}

/// The heap's barrier store of `v` into the traced field `slot` of `x`
/// (GNU R's `FIX_REFCNT` and `CHECK_OLD_TO_NEW`, then the write): what
/// `api.c`'s setters call. A cell robigo-rt made code from makes that code
/// stale (`Cx::note_code_write`, docs/g3.md §9.3: the C API's setters are
/// tracked writes). Cannot collect or exit.
///
/// # Safety
/// `x` is a live object, `slot` one of its traced pointer fields, `v` null
/// or a live object of the same heap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_rtapi_store(x: Sexp, slot: *mut Sexp, v: Sexp) {
    // SAFETY: the caller's contract: a live object's header.
    if unsafe { x.extra() } & (robigo_heap::extra::cell::AST_SOURCE | robigo_heap::extra::cell::FORMALS_SOURCE) != 0 {
        // (a null store marks nothing new: `x` stands in for it)
        let w = if v.is_null() { x } else { v };
        // SAFETY: called from C code of the thread's isolate, with `x` and
        // `w` live objects of its heap (the caller's contract).
        unsafe { robigo_rtcore::enter_leaf(|cx| cx.note_code_write(Ref::from_sexp(x, cx), Ref::from_sexp(w, cx))) };
    }
    // SAFETY: the caller's contract.
    unsafe { x.store(slot, v) }
}

/// `PRINTNAME(x)` of a symbol, which `api.c`'s `PRINTNAME` and `CAR` call:
/// the isolate's CHARSXP of the name, as Rust's `Sym::name` gives it.
/// Symbols are the process's (H5), so their own CAR is the arena's copy of
/// the name, which C must never see. Cannot collect or exit.
///
/// # Safety
/// `x` is a live SYMSXP, from C code a native entry of the thread's
/// isolate runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_rtapi_printname(x: Sexp) -> Sexp {
    // SAFETY: the caller's contract.
    unsafe {
        robigo_rtcore::enter_leaf(|cx| {
            let s = Ref::from_sexp(x, cx).as_sym().expect("robigo: PRINTNAME of a symbol");
            Ref::from(s.name(cx)).sexp()
        })
    }
}
