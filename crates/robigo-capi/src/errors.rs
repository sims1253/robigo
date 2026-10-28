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

/// What a context restores when a jump reaches it (context.c begincontext
/// / R_restore_globals): the protect stack depth, its size (the red zone
/// R_signal_protect_error opens), the R_alloc stack, the collector's
/// inhibit count, visibility and interrupt suspension. The host's own
/// state (evaluation depth, handler and restart stacks) unwinds through
/// its Rust frames, which jumps never cross: every entry into C code is a
/// C trampoline, and host callbacks return flow tokens.
pub struct Ctx {
    pptop: c_int,
    ppsize: c_int,
    vmax: *mut c_void,
    gc_inhibit: u32,
    visible: c_int,
    intr_susp: c_int,
}

pub unsafe fn ctx_save() -> Ctx {
    Ctx {
        pptop: R_PPStackTop,
        ppsize: crate::state::R_PPStackSize,
        vmax: crate::memory::vmaxget(),
        gc_inhibit: crate::state::core().gc_inhibit,
        visible: crate::state::R_Visible,
        intr_susp: crate::state::R_interrupts_suspended,
    }
}

pub unsafe fn ctx_restore(c: &Ctx) {
    R_PPStackTop = c.pptop;
    crate::state::R_PPStackSize = c.ppsize;
    crate::memory::vmaxset(c.vmax);
    crate::state::core().gc_inhibit = c.gc_inhibit;
    crate::state::R_Visible = c.visible;
    crate::state::R_interrupts_suspended = c.intr_susp;
}

/// After a trampoline returned 1: restore the context, take the token.
unsafe fn unwound(c: &Ctx) -> usize {
    ctx_restore(c);
    let t = PENDING;
    PENDING = 0;
    t
}

/// `f(data)` in a context (C trampoline).
pub unsafe fn try_void_ptr(f: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> Result<(), usize> {
    let c = ctx_save();
    if unwind::robigo_try_void_ptr(f, data) == 0 { Ok(()) } else { Err(unwound(&c)) }
}

/// `f(data)` returning a SEXP, in a context.
pub unsafe fn try_sexp_ptr(f: unsafe extern "C" fn(*mut c_void) -> Sexp, data: *mut c_void) -> Result<Sexp, usize> {
    let c = ctx_save();
    let mut out: *mut c_void = std::ptr::null_mut();
    let f: unsafe extern "C" fn(*mut c_void) -> *mut c_void = std::mem::transmute(f);
    if unwind::robigo_try_sexp_ptr(f, data, &mut out) == 0 { Ok(Sexp(out as *mut u8)) } else { Err(unwound(&c)) }
}

/// `f(data, jump)` in a context.
pub unsafe fn try_void_ptr_int(
    f: unsafe extern "C" fn(*mut c_void, c_int),
    data: *mut c_void,
    jump: c_int,
) -> Result<(), usize> {
    let c = ctx_save();
    if unwind::robigo_try_void_ptr_int(f, data, jump) == 0 { Ok(()) } else { Err(unwound(&c)) }
}

/// `f(args[0], ..., args[64])` in a context.
pub unsafe fn try_call65(f: *const c_void, args: &mut [Sexp; 65]) -> Result<Sexp, usize> {
    let c = ctx_save();
    let mut out: *mut c_void = std::ptr::null_mut();
    if unwind::robigo_try_call65(f, args.as_mut_ptr() as *mut *mut c_void, &mut out) == 0 {
        Ok(Sexp(out as *mut u8))
    } else {
        Err(unwound(&c))
    }
}

/// Run Rust code that may jump (it calls the API) in a context. Only for
/// closures that capture plain data; prefer the C trampolines for calls
/// into package code.
pub unsafe fn protect_run(f: impl FnOnce()) -> Option<usize> {
    let c = ctx_save();
    if unwind::try_run(f) { None } else { Some(unwound(&c)) }
}

// R: context.c:R_ToplevelExec — errors are reported and stop here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ToplevelExec(fun: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> c_int {
    let mut out = Sexp::NULL;
    crate::host::try_op("toplevelBegin\0", &[], &mut out);
    let r = try_void_ptr(fun, data);
    crate::host::try_op("toplevelEnd\0", &[], &mut out);
    match r {
        Ok(()) => 1,
        Err(t) => {
            (host().report)(t);
            0
        }
    }
}

// R: context.c:R_ExecWithCleanup — the cleanup runs on both paths; a
// jump out of the cleanup itself wins.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ExecWithCleanup(
    fun: unsafe extern "C" fn(*mut c_void) -> Sexp,
    data: *mut c_void,
    cleanfun: unsafe extern "C" fn(*mut c_void),
    cleandata: *mut c_void,
) -> Sexp {
    let r = try_sexp_ptr(fun, data);
    if let Err(t2) = try_void_ptr(cleanfun, cleandata) {
        if let Err(t) = r {
            (host().discard)(t);
        }
        jump_host(t2)
    }
    match r {
        Ok(v) => v,
        Err(t) => jump_host(t),
    }
}

// R: context.c:R_MakeUnwindCont
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_MakeUnwindCont() -> Sexp {
    crate::state::core().alloc_node(LISTSXP, nil(), nil(), nil())
}

/// The continuation owns the pending condition: its car is a HOST proxy
/// whose handle is the flow token and whose release discards it, so a
/// continuation that is never continued does not leak the condition.
unsafe fn cont_set(cont: Sexp, token: usize) {
    let p = crate::state::core().alloc_host(RAWSXP, token, Some(host().discard));
    cont.set_car_raw(p);
}

unsafe fn cont_take(cont: Sexp) -> usize {
    let p = cont.car();
    if !p.has_flag(robigo_core::flags::HOST) {
        return 0;
    }
    let t = robigo_core::Core::host_handle(p);
    // the proxy's release would discard the token: disarm it
    *(p.0.add(48) as *mut usize) = 0;
    cont.set_car_raw(nil());
    t
}

// R: context.c:R_UnwindProtect — the context is popped before cleanfun
// runs; cleanfun may jump away (cpp11, Rcpp), and R_ContinueUnwind may be
// called later from another frame.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_UnwindProtect(
    fun: unsafe extern "C" fn(*mut c_void) -> Sexp,
    data: *mut c_void,
    cleanfun: unsafe extern "C" fn(*mut c_void, c_int),
    cleandata: *mut c_void,
    cont: Sexp,
) -> Sexp {
    crate::memory::Rf_protect(cont);
    let r = try_sexp_ptr(fun, data);
    let jumped = r.is_err();
    if let Err(t) = r {
        cont_set(cont, t);
    }
    crate::memory::Rf_unprotect(1);
    if let Err(t2) = try_void_ptr_int(cleanfun, cleandata, jumped as c_int) {
        jump_host(t2)
    }
    match r {
        Ok(v) => v,
        Err(_) => cont,
    }
}

// R: context.c:R_ContinueUnwind
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ContinueUnwind(cont: Sexp) -> ! {
    let t = cont_take(cont);
    if t == 0 {
        error_str("bad continuation")
    }
    jump_host(t)
}
