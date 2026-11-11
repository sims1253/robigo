//! Errors, warnings, console output and non-local exits (errors.c,
//! context.c, printutils.c's Rprintf).
//!
//! An R error, a warning that became one, or any other non-local exit
//! (`return`, a restart, an interrupt) is an [`Unwind`]: a host flow
//! token. Rust code never jumps. It returns `Res<T>` and propagates with
//! `?`. Only the thin `extern "C"` wrappers that C calls turn an `Err`
//! into GNU R's `longjmp` to the innermost context, with [`ret`] or
//! [`throw`]. C code that may jump (package code, TRE) is called through
//! a C trampoline that turns its jump back into an `Err`. So a `longjmp`
//! crosses no Rust frame except the wrapper's own (`docs/core.md` §5);
//! `tests/no_inner_jumps.rs` checks the rule.

use crate::host::host;
use crate::state::{R_NilValue, R_PPStackTop, nil};
use robigo_core::Sexp;
use robigo_core::types::*;
use robigo_core::unwind;
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::num::NonZeroUsize;

unsafe extern "C" {
    fn vsnprintf(buf: *mut c_char, n: usize, fmt: *const c_char, ap: std::ffi::VaList) -> c_int;
}

const BUFSIZE: usize = 8192;

thread_local! {
    /// The flow token of the jump in progress.
    static PENDING: Cell<usize> = const { Cell::new(0) };
    static SCRATCH: std::cell::UnsafeCell<[u8; BUFSIZE]> = const { std::cell::UnsafeCell::new([0; BUFSIZE]) };
}

/// A pending non-local exit: the host's flow token for an R error, a
/// warning turned error, `return`/`break`, a restart or an interrupt. The
/// host keeps the condition until the token is continued (an export
/// wrapper jumps with it, a context hands it back to the host) or
/// discarded.
#[must_use = "a pending exit must reach an export wrapper or a context"]
#[derive(Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Unwind(NonZeroUsize);

/// The result of capi code that can exit non-locally.
pub type Res<T> = Result<T, Unwind>;

impl Unwind {
    /// The exit behind a host flow token (nonzero).
    #[inline]
    pub fn from_token(t: usize) -> Unwind {
        match NonZeroUsize::new(t) {
            Some(t) => Unwind(t),
            None => {
                eprintln!("robigo: a non-local exit without a flow token");
                std::process::abort()
            }
        }
    }

    /// The host flow token, handed over to whoever continues or
    /// discards it.
    #[inline]
    pub fn into_token(self) -> usize {
        self.0.get()
    }
}

/// A host callback's result: 0 is success, anything else the token of
/// the exit it took.
#[inline(always)]
pub fn flow(t: usize) -> Res<()> {
    match NonZeroUsize::new(t) {
        None => Ok(()),
        Some(t) => Err(Unwind(t)),
    }
}

/// The value of an export, or the jump of its pending exit: the only way
/// out of Rust by `longjmp`.
///
/// # Safety
/// Only an `extern "C"` function that C code calls may call this, in its
/// own body (not in a closure or a helper), with nothing in its frame
/// that needs dropping: the jump skips that frame. Rust code that calls
/// the API calls the `Res` function the wrapper wraps instead
/// (`tests/no_inner_jumps.rs` checks both).
#[inline(always)]
pub unsafe fn ret<T>(r: Res<T>) -> T {
    match r {
        Ok(v) => v,
        Err(u) => throw(u),
    }
}

/// Jump with a pending exit to the innermost context, running the `cend`
/// cleanups of the C contexts begun inside it on the way (context.c
/// R_run_onexits). (The C ABI makes it a call that cannot unwind, so a
/// wrapper's error path needs no landing pad and stays out of its fast
/// path.)
///
/// # Safety
/// As for [`ret`].
#[cold]
#[inline(never)]
pub unsafe extern "C" fn throw(u: Unwind) -> ! {
    let u = crate::context::run_cends(u);
    PENDING.set(u.into_token());
    unwind::jump()
}

/// The thread's message buffer (errors.c formats into an 8 KiB stack
/// buffer; zeroing one per call cost ~8k instructions per Rf_error).
/// Messages are handed to the host, which copies them at once, before
/// anything else can format into it.
#[inline(always)]
fn scratch() -> *mut [u8; BUFSIZE] {
    SCRATCH.with(|b| b.get())
}
/// options(warning.length) default
const R_WARN_LENGTH: usize = 1000;

/// Rvsnprintf_mbcs: format into `buf` (size `n`), truncating at a UTF-8
/// character boundary. Returns the untruncated length.
unsafe fn vformat(buf: *mut [u8; BUFSIZE], n: usize, fmt: *const c_char, ap: std::ffi::VaList) -> usize {
    let buf = &mut *buf;
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

/// An R error with message `msg` (NUL-terminated) against `call`
/// (`Sexp::NULL`: the context call).
pub unsafe fn error(call: Sexp, msg: *const c_char) -> Unwind {
    Unwind::from_token((host().error)(call, msg))
}

/// An error reported against the context call (R's `error()`).
pub unsafe fn error_str(msg: &str) -> Unwind {
    error(Sexp::NULL, scratch_copy(msg))
}

/// An error reported against `call` (R's `errorcall()`).
pub unsafe fn errorcall_str(call: Sexp, msg: &str) -> Unwind {
    error(call, scratch_copy(msg))
}

/// `msg`, NUL-terminated, in the scratch buffer.
unsafe fn scratch_copy(msg: &str) -> *const c_char {
    let buf = scratch().cast::<u8>();
    let n = msg.len().min(BUFSIZE - 1);
    std::ptr::copy_nonoverlapping(msg.as_ptr(), buf, n);
    *buf.add(n) = 0;
    buf as *const c_char
}

/// "robigo: C API function 'X' is not implemented yet" (porting contract §6).
pub unsafe fn not_implemented(name: &str) -> Unwind {
    error_str(&format!("robigo: C API function '{name}' is not implemented yet"))
}

// R: errors.c:error
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_error(fmt: *const c_char, args: ...) -> ! {
    let buf = scratch();
    vformat(buf, R_WARN_LENGTH, fmt, args);
    throw(error(Sexp::NULL, buf as *const c_char))
}

// R: errors.c:errorcall
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_errorcall(call: Sexp, fmt: *const c_char, args: ...) -> ! {
    let buf = scratch();
    vformat(buf, BUFSIZE, fmt, args);
    let call = if call == crate::state::R_CurrentExpression { Sexp::NULL } else { call };
    throw(error(call, buf as *const c_char))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_errorcall_cpy(call: Sexp, fmt: *const c_char, args: ...) -> ! {
    let buf = scratch();
    vformat(buf, BUFSIZE, fmt, args);
    throw(error(call, buf as *const c_char))
}

/// RprintTrunc and the trailing-newline strip of warning().
unsafe fn warning_buf(buf: *mut [u8; BUFSIZE], truncated: bool) {
    let buf = &mut *buf;
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

/// Signal a warning: it can exit (options(warn = 2), a calling handler
/// that invokes a restart).
unsafe fn do_warning(call: Sexp, buf: *const [u8; BUFSIZE], immediate: c_int) -> Res<()> {
    flow((host().warning)(call, buf as *const c_char, immediate))
}

/// nmath's `MATHLIB_WARNING`s (robigo-numerics queues them; the runtime
/// drains the queue after its own calls): R `warning()`s, as GNU R's nmath
/// calls warning() for C callers too.
#[inline]
pub unsafe fn nmath_warnings() -> Res<()> {
    if robigo_numerics::pending() { nmath_flush() } else { Ok(()) }
}

#[cold]
unsafe fn nmath_flush() -> Res<()> {
    // A warning can exit (options(warn = 2)): the queue is gone first.
    let mut r = Ok(());
    for w in robigo_numerics::take_warnings() {
        if r.is_ok() {
            r = warning_str(&w);
        }
    }
    r?;
    // R: memory.c:R_chk_calloc — nmath's failed allocation is an error
    match robigo_numerics::take_alloc_error() {
        Some(msg) => Err(error_str(&msg)),
        None => Ok(()),
    }
}

/// `warning()` with GNU R's truncation.
pub unsafe fn warning_str(msg: &str) -> Res<()> {
    let buf = scratch();
    // as Rvsnprintf_mbcs, never cutting a character
    let mut n = msg.len().min(R_WARN_LENGTH);
    while !msg.is_char_boundary(n) {
        n -= 1;
    }
    std::ptr::copy_nonoverlapping(msg.as_ptr(), buf.cast::<u8>(), n);
    *buf.cast::<u8>().add(n) = 0;
    warning_buf(buf, msg.len() > R_WARN_LENGTH);
    do_warning(Sexp::NULL, buf, 0)
}

/// `warning()` with a C format and its arguments (the formatting half of
/// [`Rf_warning`], for Rust callers that hold a `va_list`).
unsafe fn vwarning(fmt: *const c_char, args: std::ffi::VaList) -> Res<()> {
    let buf = scratch();
    let psize = R_WARN_LENGTH + 1;
    let n = vformat(buf, psize, fmt, args);
    warning_buf(buf, n >= psize);
    do_warning(Sexp::NULL, buf, 0)
}

// R: errors.c:warning
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_warning(fmt: *const c_char, args: ...) {
    ret(vwarning(fmt, args))
}

// R: errors.c:warningcall
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_warningcall(call: Sexp, fmt: *const c_char, args: ...) {
    let buf = scratch();
    vformat(buf, BUFSIZE, fmt, args);
    ret(do_warning(if call.is_null() { nil() } else { call }, buf, 0))
}

// R: errors.c:warningcall_immediate
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_warningcall_immediate(call: Sexp, fmt: *const c_char, args: ...) {
    let buf = scratch();
    vformat(buf, BUFSIZE, fmt, args);
    ret(do_warning(if call.is_null() { nil() } else { call }, buf, 1))
}

// R: errors.c:R_BadLongVector
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_BadLongVector(x: Sexp, file: *const c_char, line: c_int) -> ! {
    throw(bad_long_vector(x, file, line))
}

pub unsafe fn bad_long_vector(_x: Sexp, file: *const c_char, line: c_int) -> Unwind {
    let f = CStr::from_ptr(file).to_string_lossy();
    error_str(&format!("long vectors not supported yet: {f}:{line}"))
}

// R: memory.c:R_signal_protect_error (the condition is reported without a call)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_signal_protect_error() -> ! {
    throw(protect_error())
}

#[cold]
pub unsafe fn protect_error() -> Unwind {
    error(R_NilValue, c"protect(): protection stack overflow".as_ptr())
}

// R: memory.c:R_signal_unprotect_error
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_signal_unprotect_error() -> ! {
    throw(unprotect_error())
}

#[cold]
pub unsafe fn unprotect_error() -> Unwind {
    let n = R_PPStackTop;
    error_str(&if n == 1 {
        format!("unprotect(): only {n} protected item")
    } else {
        format!("unprotect(): only {n} protected items")
    })
}

// R: memory.c:R_signal_reprotect_error
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_signal_reprotect_error(i: c_int) -> ! {
    throw(reprotect_error(i))
}

#[cold]
pub unsafe fn reprotect_error(i: c_int) -> Unwind {
    let n = R_PPStackTop;
    error_str(&if n == 1 {
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

// R: unix/system.c:R_Busy — ptr_R_Busy, which is unix/sys-std.c:Rstd_Busy
// for the standard interface: it ignores the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_Busy(_which: c_int) {}

// R: errors.c:R_CheckUserInterrupt — robigo has no asynchronous
// interrupts yet; the flag is honoured if a package sets it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_CheckUserInterrupt() {
    ret(check_user_interrupt())
}

pub unsafe fn check_user_interrupt() -> Res<()> {
    if crate::state::R_interrupts_pending != 0 && crate::state::R_interrupts_suspended == 0 { onintr() } else { Ok(()) }
}

// R: errors.c:onintr — jumps to top level like a user interrupt; while
// interrupts are suspended it leaves the interrupt pending (onintrEx).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_onintr() {
    ret(onintr())
}

pub unsafe fn onintr() -> Res<()> {
    if crate::state::R_interrupts_suspended != 0 {
        crate::state::R_interrupts_pending = 1;
        return Ok(());
    }
    crate::state::R_interrupts_pending = 0;
    let mut out = Sexp::NULL;
    flow(crate::host::try_op("onintr\0", &[], &mut out))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_onintrNoResume() {
    ret(onintr())
}

// ---- contexts (context.c) ------------------------------------------------

/// What a context restores when a jump reaches it (context.c begincontext
/// / R_restore_globals): the protect stack depth, its size (the red zone
/// R_signal_protect_error opens), the R_alloc stack, the collector's
/// inhibit count, visibility, interrupt suspension, and the host's own
/// state (its condition handler and restart stacks). The rest of the
/// host's state (evaluation depth) unwinds through its Rust frames, which
/// jumps never cross.
pub struct Ctx {
    host: [usize; 2],
    pptop: c_int,
    ppsize: c_int,
    vmax: *mut c_void,
    gc_inhibit: u32,
    visible: c_int,
    intr_susp: c_int,
}

pub unsafe fn ctx_save() -> Ctx {
    let s = crate::state::st();
    let mut host = [0usize; 2];
    if let Some(h) = &s.host {
        (h.ctx_save)(host.as_mut_ptr());
    }
    Ctx {
        host,
        pptop: R_PPStackTop,
        ppsize: crate::state::R_PPStackSize,
        vmax: s.vstack.0 as *mut c_void,
        gc_inhibit: s.core.gc_inhibit,
        visible: crate::state::R_Visible,
        intr_susp: crate::state::R_interrupts_suspended,
    }
}

/// The context was left normally: release the host's saved state.
pub unsafe fn ctx_done(c: &Ctx) {
    if let Some(h) = &crate::state::st().host {
        (h.ctx_drop)(c.host.as_ptr());
    }
}

pub unsafe fn ctx_restore(c: &Ctx) {
    let s = crate::state::st();
    if let Some(h) = &s.host {
        (h.ctx_restore)(c.host.as_ptr());
    }
    R_PPStackTop = c.pptop;
    crate::state::R_PPStackSize = c.ppsize;
    s.vstack = if c.vmax.is_null() { crate::state::R_NilValue } else { Sexp(c.vmax as *mut u8) };
    s.core.gc_inhibit = c.gc_inhibit;
    crate::state::R_Visible = c.visible;
    crate::state::R_interrupts_suspended = c.intr_susp;
}

/// After a shim trampoline returned 1: the exit its jump carried.
#[inline]
fn jumped() -> Unwind {
    Unwind::from_token(PENDING.replace(0))
}

/// Run `f` as a context: if it fails, what the context saved is
/// restored first (as a jump reaching it would), whether the exit came
/// from a jump out of C code or from Rust code's `Err`. The caller then
/// handles the exit (reports it, discards it, turns it into the host's
/// condition) or passes it on.
pub unsafe fn in_context<T>(f: impl FnOnce() -> Res<T>) -> Res<T> {
    let c = ctx_save();
    let r = f();
    match &r {
        Ok(_) => ctx_done(&c),
        Err(_) => ctx_restore(&c),
    }
    r
}

/// `f(data)`, package code, through a C trampoline: its jump becomes an
/// `Err`. Pass-through: nothing is restored, so the caller passes the
/// exit on or runs this inside [`in_context`].
#[inline]
pub unsafe fn call_void_ptr(f: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> Res<()> {
    if unwind::robigo_try_void_ptr(f, data) == 0 { Ok(()) } else { Err(jumped()) }
}

/// `f(data)` returning a SEXP, as [`call_void_ptr`].
#[inline]
pub unsafe fn call_sexp_ptr(f: unsafe extern "C" fn(*mut c_void) -> Sexp, data: *mut c_void) -> Res<Sexp> {
    let mut out: *mut c_void = std::ptr::null_mut();
    let f: unsafe extern "C" fn(*mut c_void) -> *mut c_void = std::mem::transmute(f);
    if unwind::robigo_try_sexp_ptr(f, data, &mut out) == 0 { Ok(Sexp(out as *mut u8)) } else { Err(jumped()) }
}

/// `f(data, jump)`, as [`call_void_ptr`].
#[inline]
pub unsafe fn call_void_ptr_int(
    f: unsafe extern "C" fn(*mut c_void, c_int),
    data: *mut c_void,
    jump: c_int,
) -> Res<()> {
    if unwind::robigo_try_void_ptr_int(f, data, jump) == 0 { Ok(()) } else { Err(jumped()) }
}

/// `f(args[0], ..., args[64])`, as [`call_void_ptr`]: a package routine
/// (`.Call`, `.External`, `.C`), whose arguments the caller protected.
///
/// While it runs, the stack scan's open segment starts at the C
/// trampoline's frame, not at the bridge's entry ([`crate::bridge`]'s
/// `enter`), and the stack below the trampoline is zeroed first
/// (`unwind::robigo_try_call65`): the bridge frames hold objects only
/// through precise roots (the arguments are on the protect stack, the
/// result is written after the return), and the words they leave
/// unwritten are stale ones of earlier frames at that depth. A stale
/// external pointer in a trampoline's never-written `jmp_buf` signal mask,
/// and then in this function's frame (inlined into its caller's), kept it
/// from being finalized (capi weak-keys-collections,
/// native_call_frames_keep_nothing; ledger 2026-11-11).
///
/// # Safety
/// `f` is a C function taking 65 pointer-sized arguments. Every slot of
/// `args` that holds an object must stay reachable through a precise root
/// (the protect stack, as `robigo_dotcall` arranges, or the evaluator) for
/// the whole call: while the routine runs, the trampoline's frame is the
/// top of the scan's open segment, so the caller's frames are not scanned
/// and do not root those words. Slots holding words that are not objects
/// (`.C`'s data pointers) need no root.
#[inline]
pub unsafe fn call65(f: *const c_void, args: &mut [Sexp; 65]) -> Res<Sexp> {
    let mut out: *mut c_void = std::ptr::null_mut();
    let scan_top = crate::state::core().scan_top_slot();
    let t = unwind::robigo_try_call65(f, args.as_mut_ptr() as *mut *mut c_void, &mut out, scan_top);
    if t == 0 { Ok(Sexp(out as *mut u8)) } else { Err(jumped()) }
}

/// `f(a[0], ..., a[5])` returning a word (a pointer, or an integer in
/// its low bits), for package callbacks of up to six word arguments
/// (ALTREP methods, TRE), as [`call_void_ptr`].
///
/// # Safety
/// `f` must take at most six integer or pointer arguments, which `a`
/// holds in order (the rest are ignored), and return such a value or
/// nothing.
#[inline]
pub unsafe fn call_w6(f: usize, a: [usize; 6]) -> Res<usize> {
    let mut out = 0usize;
    if unwind::robigo_try_w6(f as *const c_void, a.as_ptr(), &mut out) == 0 { Ok(out) } else { Err(jumped()) }
}

/// `f(a0, a1)` returning a double, as [`call_w6`].
#[inline]
pub unsafe fn call_d2(f: usize, a0: usize, a1: usize) -> Res<f64> {
    let mut out = 0f64;
    if unwind::robigo_try_d2(f as *const c_void, a0, a1, &mut out) == 0 { Ok(out) } else { Err(jumped()) }
}

/// `f(a0, a1)` returning an `Rcomplex`, as [`call_w6`].
#[inline]
pub unsafe fn call_c2(f: usize, a0: usize, a1: usize) -> Res<robigo_core::Rcomplex> {
    let mut out = robigo_core::Rcomplex { r: 0.0, i: 0.0 };
    if unwind::robigo_try_c2(f as *const c_void, a0, a1, (&raw mut out).cast()) == 0 { Ok(out) } else { Err(jumped()) }
}

// R: context.c:R_ToplevelExec — errors are reported and stop here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ToplevelExec(fun: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> c_int {
    let mut out = Sexp::NULL;
    crate::host::try_op("toplevelBegin\0", &[], &mut out);
    let r = in_context(|| call_void_ptr(fun, data));
    crate::host::try_op("toplevelEnd\0", &[], &mut out);
    match r {
        Ok(()) => 1,
        Err(u) => {
            (host().report)(u.into_token());
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
    ret(exec_with_cleanup(fun, data, cleanfun, cleandata))
}

unsafe fn exec_with_cleanup(
    fun: unsafe extern "C" fn(*mut c_void) -> Sexp,
    data: *mut c_void,
    cleanfun: unsafe extern "C" fn(*mut c_void),
    cleandata: *mut c_void,
) -> Res<Sexp> {
    let r = in_context(|| call_sexp_ptr(fun, data));
    if let Err(u2) = in_context(|| call_void_ptr(cleanfun, cleandata)) {
        if let Err(u) = r {
            (host().discard)(u.into_token());
        }
        return Err(u2);
    }
    r
}

// R: context.c:R_MakeUnwindCont
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_MakeUnwindCont() -> Sexp {
    crate::state::core().alloc_node(LISTSXP, nil(), nil(), nil())
}

/// The continuation owns the pending condition: its car is a HOST proxy
/// whose handle is the flow token and whose release discards it, so a
/// continuation that is never continued does not leak the condition.
unsafe fn cont_set(cont: Sexp, u: Unwind) {
    let p = crate::state::core().alloc_host(RAWSXP, u.into_token(), Some(host().discard));
    cont.set_car(p);
}

unsafe fn cont_take(cont: Sexp) -> usize {
    let p = cont.car();
    if !p.has_flag(robigo_core::flags::HOST) {
        return 0;
    }
    let t = robigo_core::Core::host_handle(p);
    // the proxy's release would discard the token: disarm it
    *(p.0.add(48) as *mut usize) = 0;
    cont.set_car(nil());
    t
}

/// `fun(data)` in a context; a jump out of it is stored in `cont`
/// (protected meanwhile). `Ok(Err(()))`: `fun` jumped.
unsafe fn unwind_protect_fun(
    fun: unsafe extern "C" fn(*mut c_void) -> Sexp,
    data: *mut c_void,
    cont: Sexp,
) -> Res<Result<Sexp, ()>> {
    crate::memory::protect(cont)?;
    let r = match in_context(|| call_sexp_ptr(fun, data)) {
        Ok(v) => Ok(v),
        Err(u) => {
            cont_set(cont, u);
            Err(())
        }
    };
    crate::memory::unprotect(1)?;
    Ok(r)
}

// R: context.c:R_UnwindProtect — the context is popped before cleanfun
// runs. After a jump the continuation receives the pending exit, cleanfun
// runs with jump = TRUE, and the unwind continues (R_ContinueUnwind):
// R_UnwindProtect never returns after a jump. cleanfun is called
// directly from this frame, outside any robigo context: cpp11 and Rcpp
// leave it by longjmp to their own setjmp (keeping the exit pending in
// the continuation for a later R_ContinueUnwind), which must not skip a
// context of ours, nor a Rust frame other than this wrapper's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_UnwindProtect(
    fun: unsafe extern "C" fn(*mut c_void) -> Sexp,
    data: *mut c_void,
    cleanfun: unsafe extern "C-unwind" fn(*mut c_void, c_int),
    cleandata: *mut c_void,
    cont: Sexp,
) -> Sexp {
    // Simple usage with a NULL continuation token: it stays protected
    // while cleanfun runs, as in GNU R's recursive call.
    let own = cont.is_null();
    let cont = if own { ret(crate::memory::protect(R_MakeUnwindCont())) } else { cont };
    let r = ret(unwind_protect_fun(fun, data, cont));
    cleanfun(cleandata, r.is_err() as c_int);
    match r {
        Ok(v) => {
            if own {
                ret(crate::memory::unprotect(1));
            }
            v
        }
        Err(()) => throw(continue_unwind(cont)),
    }
}

// R: context.c:R_ContinueUnwind
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ContinueUnwind(cont: Sexp) -> ! {
    throw(continue_unwind(cont))
}

unsafe fn continue_unwind(cont: Sexp) -> Unwind {
    match cont_take(cont) {
        0 => error_str("bad continuation"),
        t => Unwind::from_token(t),
    }
}
