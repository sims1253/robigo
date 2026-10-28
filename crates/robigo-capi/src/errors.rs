//! Errors, warnings, console output and non-local exits (errors.c,
//! context.c, printutils.c's Rprintf). An error becomes a host flow
//! token, then `longjmp`s to the innermost context (`docs/core.md` §5).

use crate::host::host;
use crate::state::{R_NilValue, R_PPStackTop, nil};
use robigo_core::Sexp;
use robigo_core::types::*;
use robigo_core::unwind;
use std::ffi::{CStr, c_char, c_int, c_void};

/// The flow token of the jump in progress.
pub static mut PENDING: usize = 0;

/// Continuation tokens of R_UnwindProtect (`R_MakeUnwindCont`) are
/// nodes; their car holds the pending flow token as an integer.
pub unsafe fn mark_roots(_c: &mut robigo_core::Core) {}

unsafe extern "C" {
    fn vsnprintf(buf: *mut c_char, n: usize, fmt: *const c_char, ap: std::ffi::VaList) -> c_int;
}

const BUFSIZE: usize = 8192;
/// options(warning.length) default
const R_WARN_LENGTH: usize = 1000;

/// Rvsnprintf_mbcs: format into `buf` (size `n`), truncating at a UTF-8
/// character boundary. Returns the untruncated length.
unsafe fn vformat(buf: &mut [u8], n: usize, fmt: *const c_char, ap: std::ffi::VaList) -> usize {
    let n = n.min(buf.len());
    let r = vsnprintf(buf.as_mut_ptr() as *mut c_char, n, fmt, ap);
    let len = CStr::from_ptr(buf.as_ptr() as *const c_char).to_bytes().len();
    if r >= 0 && r as usize >= n {
        // trim an incomplete trailing UTF-8 sequence
        let mut end = len;
        while end > 0 && buf[end - 1] & 0xC0 == 0x80 {
            end -= 1;
        }
        if end > 0 && buf[end - 1] >= 0xC0 {
            let need = if buf[end - 1] >= 0xF0 {
                4
            } else if buf[end - 1] >= 0xE0 {
                3
            } else {
                2
            };
            if len - (end - 1) < need {
                buf[end - 1] = 0;
            }
        }
    }
    r.max(0) as usize
}

/// Jump with a host flow token. Plain frames only (see `unwind`).
pub unsafe fn jump_host(token: usize) -> ! {
    PENDING = token;
    unwind::jump()
}

/// Raise an R error with message `msg` (NUL-terminated) against `call`
/// (`Sexp::NULL`: the context call).
pub unsafe fn raise(call: Sexp, msg: *const c_char) -> ! {
    let t = (host().error)(call, msg);
    jump_host(t)
}

/// Raise an error from a Rust message.
pub unsafe fn error_str(msg: &str) -> ! {
    let mut buf = [0u8; BUFSIZE];
    let n = msg.len().min(BUFSIZE - 1);
    buf[..n].copy_from_slice(&msg.as_bytes()[..n]);
    raise(Sexp::NULL, buf.as_ptr() as *const c_char)
}

/// Raise an error reported against `call`.
pub unsafe fn errorcall_str(call: Sexp, msg: &str) -> ! {
    let mut buf = [0u8; BUFSIZE];
    let n = msg.len().min(BUFSIZE - 1);
    buf[..n].copy_from_slice(&msg.as_bytes()[..n]);
    raise(call, buf.as_ptr() as *const c_char)
}

/// "robigo: C API function 'X' is not implemented yet" (porting contract §6).
pub unsafe fn not_implemented(name: &str) -> ! {
    let mut buf = [0u8; 256];
    let s = format!("robigo: C API function '{name}' is not implemented yet");
    let n = s.len().min(255);
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    drop(s);
    raise(Sexp::NULL, buf.as_ptr() as *const c_char)
}

// R: errors.c:error
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_error(fmt: *const c_char, args: ...) -> ! {
    let mut buf = [0u8; BUFSIZE];
    vformat(&mut buf, R_WARN_LENGTH, fmt, args);
    raise(Sexp::NULL, buf.as_ptr() as *const c_char)
}

// R: errors.c:errorcall
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_errorcall(call: Sexp, fmt: *const c_char, args: ...) -> ! {
    let mut buf = [0u8; BUFSIZE];
    vformat(&mut buf, BUFSIZE, fmt, args);
    let call = if call == crate::state::R_CurrentExpression { Sexp::NULL } else { call };
    raise(call, buf.as_ptr() as *const c_char)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_errorcall_cpy(call: Sexp, fmt: *const c_char, args: ...) -> ! {
    let mut buf = [0u8; BUFSIZE];
    vformat(&mut buf, BUFSIZE, fmt, args);
    raise(call, buf.as_ptr() as *const c_char)
}

/// RprintTrunc and the trailing-newline strip of warning().
unsafe fn warning_buf(buf: &mut [u8; BUFSIZE], truncated: bool) {
    let len = CStr::from_ptr(buf.as_ptr() as *const c_char).to_bytes().len();
    if len > 0 && buf[len - 1] == b'\n' {
        buf[len - 1] = 0;
    }
    if truncated {
        let len = CStr::from_ptr(buf.as_ptr() as *const c_char).to_bytes().len();
        let add = b" [... truncated]\0";
        buf[len..len + add.len()].copy_from_slice(add);
    }
}

unsafe fn do_warning(call: Sexp, buf: &[u8], immediate: c_int) {
    let t = (host().warning)(call, buf.as_ptr() as *const c_char, immediate);
    if t != 0 {
        jump_host(t);
    }
}

// R: errors.c:warning
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_warning(fmt: *const c_char, args: ...) {
    let mut buf = [0u8; BUFSIZE];
    let psize = R_WARN_LENGTH + 1;
    let n = vformat(&mut buf, psize, fmt, args);
    warning_buf(&mut buf, n >= psize);
    do_warning(Sexp::NULL, &buf, 0)
}

// R: errors.c:warningcall
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_warningcall(call: Sexp, fmt: *const c_char, args: ...) {
    let mut buf = [0u8; BUFSIZE];
    vformat(&mut buf, BUFSIZE, fmt, args);
    do_warning(if call.is_null() { nil() } else { call }, &buf, 0)
}

// R: errors.c:warningcall_immediate
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_warningcall_immediate(call: Sexp, fmt: *const c_char, args: ...) {
    let mut buf = [0u8; BUFSIZE];
    vformat(&mut buf, BUFSIZE, fmt, args);
    do_warning(if call.is_null() { nil() } else { call }, &buf, 1)
}

// R: errors.c:R_BadLongVector
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_BadLongVector(_x: Sexp, file: *const c_char, line: c_int) -> ! {
    let f = CStr::from_ptr(file).to_string_lossy().into_owned();
    let msg = format!("long vectors not supported yet: {f}:{line}");
    drop(f);
    error_str_owned(msg)
}

/// Raise with an owned message (moved into a fixed buffer first so no
/// `Drop` is pending across the jump).
pub unsafe fn error_str_owned(msg: String) -> ! {
    let mut buf = [0u8; BUFSIZE];
    let n = msg.len().min(BUFSIZE - 1);
    buf[..n].copy_from_slice(&msg.as_bytes()[..n]);
    drop(msg);
    raise(Sexp::NULL, buf.as_ptr() as *const c_char)
}

pub unsafe fn errorcall_owned(call: Sexp, msg: String) -> ! {
    let mut buf = [0u8; BUFSIZE];
    let n = msg.len().min(BUFSIZE - 1);
    buf[..n].copy_from_slice(&msg.as_bytes()[..n]);
    drop(msg);
    raise(call, buf.as_ptr() as *const c_char)
}

// R: memory.c:R_signal_protect_error (the condition is reported without a call)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_signal_protect_error() -> ! {
    raise(R_NilValue, c"protect(): protection stack overflow".as_ptr())
}

// R: memory.c:R_signal_unprotect_error
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_signal_unprotect_error() -> ! {
    let n = R_PPStackTop;
    error_str_owned(if n == 1 {
        format!("unprotect(): only {n} protected item")
    } else {
        format!("unprotect(): only {n} protected items")
    })
}

// R: memory.c:R_signal_reprotect_error
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_signal_reprotect_error(i: c_int) -> ! {
    let n = R_PPStackTop;
    error_str_owned(if n == 1 {
        format!("R_Reprotect: only {n} protected item, can't reprotect index {i}")
    } else {
        format!("R_Reprotect: only {n} protected items, can't reprotect index {i}")
    })
}

// ---- console output (printutils.c) --------------------------------------

unsafe fn vprint(stream: c_int, fmt: *const c_char, ap: std::ffi::VaList) {
    // R: printutils.c:Rcons_vprintf, a 8192-byte buffer then malloc
    let mut buf = vec![0u8; BUFSIZE];
    let mut ap2 = ap.clone();
    let n = vsnprintf(buf.as_mut_ptr() as *mut c_char, BUFSIZE, fmt, ap);
    if n >= 0 && n as usize >= BUFSIZE {
        buf = vec![0u8; n as usize + 1];
        vsnprintf(buf.as_mut_ptr() as *mut c_char, n as usize + 1, fmt, ap2.clone());
    }
    let _ = &mut ap2;
    let len = CStr::from_ptr(buf.as_ptr() as *const c_char).to_bytes().len();
    crate::host::write(stream, &buf[..len]);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rprintf(fmt: *const c_char, args: ...) {
    vprint(0, fmt, args)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn REprintf(fmt: *const c_char, args: ...) {
    vprint(1, fmt, args)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rvprintf(fmt: *const c_char, ap: std::ffi::VaList) {
    vprint(0, fmt, ap)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn REvprintf(fmt: *const c_char, ap: std::ffi::VaList) {
    vprint(1, fmt, ap)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_FlushConsole() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ProcessEvents() {}

// R: errors.c:R_CheckUserInterrupt — robigo has no asynchronous
// interrupts yet; the flag is honoured if a package sets it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_CheckUserInterrupt() {
    if crate::state::R_interrupts_pending != 0 && crate::state::R_interrupts_suspended == 0 {
        Rf_onintr()
    }
}

// R: errors.c:onintr — jumps to top level like a user interrupt.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_onintr() {
    crate::state::R_interrupts_pending = 0;
    let mut out = Sexp::NULL;
    let t = crate::host::try_op("onintr\0", &[], &mut out);
    if t != 0 {
        jump_host(t);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_onintrNoResume() {
    Rf_onintr()
}

// ---- contexts (context.c) ------------------------------------------------

/// Run `f(data)` in a context; on a jump, returns the flow token.
pub unsafe fn protect_run(f: impl FnOnce()) -> Option<usize> {
    let top = R_PPStackTop;
    let vmax = crate::memory::vmaxget();
    if unwind::try_run(f) {
        None
    } else {
        R_PPStackTop = top;
        crate::memory::vmaxset(vmax);
        let t = PENDING;
        PENDING = 0;
        Some(t)
    }
}

// R: context.c:R_ToplevelExec — errors are reported and stop here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ToplevelExec(fun: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> c_int {
    match protect_run(|| fun(data)) {
        None => 1,
        Some(t) => {
            (host().report)(t);
            0
        }
    }
}

// R: context.c:R_ExecWithCleanup
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ExecWithCleanup(
    fun: unsafe extern "C" fn(*mut c_void) -> Sexp,
    data: *mut c_void,
    cleanfun: unsafe extern "C" fn(*mut c_void),
    cleandata: *mut c_void,
) -> Sexp {
    let mut out = Sexp::NULL;
    let o = &mut out as *mut Sexp;
    match protect_run(|| *o = fun(data)) {
        None => {
            cleanfun(cleandata);
            out
        }
        Some(t) => {
            cleanfun(cleandata);
            jump_host(t)
        }
    }
}

// R: context.c:R_MakeUnwindCont
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_MakeUnwindCont() -> Sexp {
    crate::state::core().alloc_node(LISTSXP, nil(), nil(), nil())
}

/// The flow token a continuation holds: an 8-byte RAWSXP in its car.
unsafe fn cont_token(cont: Sexp) -> usize {
    let r = cont.car();
    if r.typ() == RAWSXP && r.std_len() == 8 { *(r.std_data() as *const usize) } else { 0 }
}

unsafe fn cont_set(cont: Sexp, token: usize) {
    let r = crate::state::core().alloc_vector(RAWSXP, 8);
    *(r.std_data() as *mut usize) = token;
    cont.set_car_raw(r);
}

// R: context.c:R_UnwindProtect
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_UnwindProtect(
    fun: unsafe extern "C" fn(*mut c_void) -> Sexp,
    data: *mut c_void,
    cleanfun: unsafe extern "C" fn(*mut c_void, c_int),
    cleandata: *mut c_void,
    cont: Sexp,
) -> Sexp {
    let mut out = Sexp::NULL;
    let o = &mut out as *mut Sexp;
    match protect_run(|| *o = fun(data)) {
        None => {
            cleanfun(cleandata, 0);
            out
        }
        Some(t) => {
            cont_set(cont, t);
            cleanfun(cleandata, 1);
            cont
        }
    }
}

// R: context.c:R_ContinueUnwind
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ContinueUnwind(cont: Sexp) -> ! {
    let t = cont_token(cont);
    cont.set_car_raw(nil());
    if t == 0 {
        error_str("bad continuation")
    }
    jump_host(t)
}
