#![allow(static_mut_refs)]
//! libR.so's API driven from Rust with a stub host. The API state is bound
//! to one thread (docs/core.md §3, §10), so everything runs in one test.

use R as capi;
use capi::Sexp;
use capi::state::*;
use robigo_core::types::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};

thread_local! {
    static ERRORS: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static NEXT: RefCell<usize> = const { RefCell::new(1) };
    static WARN_IS_ERROR: RefCell<bool> = const { RefCell::new(false) };
}

fn log(s: String) {
    LOG.with(|l| l.borrow_mut().push(s));
}

fn take_log() -> Vec<String> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

fn new_token(msg: String) -> usize {
    let t = NEXT.with(|n| {
        let mut n = n.borrow_mut();
        *n += 1;
        *n
    });
    ERRORS.with(|e| e.borrow_mut().insert(t, msg));
    t
}

unsafe extern "C" fn h_eval(_e: Sexp, _r: Sexp, _o: *mut Sexp) -> usize {
    new_token("eval not available".into())
}
unsafe extern "C" fn h_write(stream: c_int, buf: *const u8, len: usize) {
    let s = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(buf, len) }).into_owned();
    log(format!("write{stream}: {s}"));
}
unsafe extern "C" fn h_warning(_call: Sexp, msg: *const c_char, _imm: c_int) -> usize {
    let m = unsafe { CStr::from_ptr(msg) }.to_string_lossy().into_owned();
    if WARN_IS_ERROR.with(|w| *w.borrow()) {
        return new_token(format!("(converted from warning) {m}"));
    }
    log(format!("warning: {m}"));
    0
}
unsafe extern "C" fn h_op(name: *const c_char, _a: *const Sexp, _n: c_int, _o: *mut Sexp) -> usize {
    let n = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    new_token(format!("op {n}"))
}
unsafe extern "C" fn h_error(_call: Sexp, msg: *const c_char) -> usize {
    new_token(unsafe { CStr::from_ptr(msg) }.to_string_lossy().into_owned())
}
unsafe extern "C" fn h_report(t: usize) {
    let m = ERRORS.with(|e| e.borrow_mut().remove(&t)).unwrap_or_default();
    log(format!("reported: {m}"));
}
unsafe extern "C" fn h_discard(t: usize) {
    ERRORS.with(|e| e.borrow_mut().remove(&t));
}

fn pending_errors() -> usize {
    ERRORS.with(|e| e.borrow().len())
}

static HOST: capi::host::Host = capi::host::Host {
    eval: h_eval,
    write: h_write,
    warning: h_warning,
    op: h_op,
    error: h_error,
    report: h_report,
    discard: h_discard,
};

unsafe extern "C" fn body_error(_: *mut c_void) {
    unsafe { capi::errors::Rf_error(c"boom %d".as_ptr(), 42) }
}

unsafe extern "C" fn body_print(_: *mut c_void) {
    unsafe { capi::errors::Rprintf(c"hello %s %.1f\n".as_ptr(), c"C".as_ptr(), 2.5) }
}

unsafe extern "C" fn fun_error(_: *mut c_void) -> Sexp {
    unsafe {
        capi::memory::Rf_protect(capi::memory::Rf_allocVector(INTSXP as u32, 3));
        capi::errors::Rf_error(c"in the body".as_ptr())
    }
}

unsafe extern "C" fn fun_ok(_: *mut c_void) -> Sexp {
    unsafe { capi::list::Rf_ScalarInteger(41) }
}

unsafe extern "C" fn cleanup(_: *mut c_void, jump: c_int) {
    log(format!("cleanup jump={jump}"));
}

unsafe extern "C" fn fun_warn(_: *mut c_void) {
    unsafe { capi::errors::Rf_warning(c"careful".as_ptr()) }
}

/// R_UnwindProtect whose body errors; the continuation is dropped
/// afterwards. Returns the number of pending conditions.
#[inline(never)]
unsafe fn unwind_and_abandon() -> usize {
    unsafe {
        let cont = capi::memory::Rf_protect(capi::errors::R_MakeUnwindCont());
        let r = capi::errors::R_UnwindProtect(fun_error, std::ptr::null_mut(), cleanup, std::ptr::null_mut(), cont);
        assert_eq!(r, cont, "unwound: the continuation comes back");
        assert_eq!(take_log(), vec!["cleanup jump=1".to_string()]);
        assert_eq!(R_PPStackTop, 1, "the body's PROTECT was undone");
        capi::memory::Rf_unprotect(1);
        let pending = pending_errors();
        assert!(pending >= 1);
        pending
    }
}

#[inline(never)]
fn clobber_stack() {
    let a = [0usize; 8192];
    std::hint::black_box(&a);
}

#[test]
fn the_api() {
    unsafe {
        capi::host::robigo_capi_init(&HOST);
        if std::env::var_os("ROBIGO_GCTORTURE").is_some() {
            capi::state::core().set_torture(true);
        }

        // ---- allocation, protect, accessors
        let x = capi::memory::Rf_protect(capi::memory::Rf_allocVector(INTSXP as u32, 5));
        for i in 0..5 {
            *capi::memory::INTEGER(x).add(i) = i as i32 * 10;
        }
        let names = capi::memory::Rf_protect(capi::memory::Rf_allocVector(STRSXP as u32, 5));
        for i in 0..5 {
            let s = std::ffi::CString::new(format!("n{i}")).unwrap();
            capi::memory::SET_STRING_ELT(names, i, capi::strings::Rf_mkChar(s.as_ptr()));
        }
        capi::attrib::Rf_setAttrib(x, R_NamesSymbol, names);
        for _ in 0..20_000 {
            capi::memory::Rf_allocVector(REALSXP as u32, 8);
        }
        let nm = capi::attrib::Rf_getAttrib(x, R_NamesSymbol);
        assert_eq!(capi::memory::LENGTH(nm), 5);
        assert_eq!(CStr::from_ptr(capi::memory::R_CHAR(capi::memory::STRING_ELT(nm, 3))).to_bytes(), b"n3");
        assert_eq!(*capi::memory::INTEGER(x).add(4), 40);
        assert_eq!(capi::strings::Rf_mkChar(c"n3".as_ptr()), capi::memory::STRING_ELT(nm, 3), "CHARSXPs are cached");
        assert_eq!(capi::memory::ATTRIB(capi::memory::STRING_ELT(nm, 3)), R_NilValue, "ATTRIB(CHARSXP)");
        capi::memory::Rf_unprotect(2);
        assert_eq!(R_PPStackTop, 0);

        // ---- Rprintf through the host
        assert_eq!(capi::errors::R_ToplevelExec(body_print, std::ptr::null_mut()), 1);
        assert_eq!(take_log(), vec!["write0: hello C 2.5\n".to_string()]);

        // ---- Rf_error caught by R_ToplevelExec: reported, stack restored
        for _ in 0..3 {
            capi::memory::Rf_protect(R_NilValue);
            let top = R_PPStackTop;
            assert_eq!(capi::errors::R_ToplevelExec(body_error, std::ptr::null_mut()), 0);
            assert_eq!(R_PPStackTop, top, "protect depth stable over repeated errors");
            capi::memory::Rf_unprotect(1);
        }
        assert_eq!(take_log(), vec!["reported: boom 42".to_string(); 3]);
        assert_eq!(capi::unwind_depth(), 0);

        // ---- R_UnwindProtect: cleanup runs, continuation owns the condition
        let pending = unwind_and_abandon();
        clobber_stack();
        // never continued: the condition dies with the continuation
        capi::host::robigo_gc();
        assert_eq!(pending_errors(), pending - 1, "an abandoned continuation discards its condition");

        let cont = capi::memory::Rf_protect(capi::errors::R_MakeUnwindCont());
        let r = capi::errors::R_UnwindProtect(fun_ok, std::ptr::null_mut(), cleanup, std::ptr::null_mut(), cont);
        assert_eq!(*capi::memory::INTEGER(r), 41);
        assert_eq!(take_log(), vec!["cleanup jump=0".to_string()]);
        capi::memory::Rf_unprotect(1);

        // ---- warnings may jump (options(warn = 2))
        assert_eq!(capi::errors::R_ToplevelExec(fun_warn, std::ptr::null_mut()), 1);
        assert_eq!(take_log(), vec!["warning: careful".to_string()]);
        WARN_IS_ERROR.with(|w| *w.borrow_mut() = true);
        assert_eq!(capi::errors::R_ToplevelExec(fun_warn, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: (converted from warning) careful".to_string()]);
        WARN_IS_ERROR.with(|w| *w.borrow_mut() = false);

        // ---- errors inside allocation helpers report GNU R's messages
        unsafe extern "C" fn neg(_: *mut c_void) {
            unsafe { capi::memory::Rf_allocVector(INTSXP as u32, -1) };
        }
        assert_eq!(capi::errors::R_ToplevelExec(neg, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: negative length vectors are not allowed".to_string()]);

        // ---- .Call through the 65-argument trampoline
        unsafe extern "C" fn add3(a: Sexp, b: Sexp, c: Sexp) -> Sexp {
            unsafe {
                capi::list::Rf_ScalarInteger(
                    *capi::memory::INTEGER(a) + *capi::memory::INTEGER(b) + *capi::memory::INTEGER(c),
                )
            }
        }
        let args = [capi::list::Rf_ScalarInteger(1), capi::list::Rf_ScalarInteger(2), capi::list::Rf_ScalarInteger(39)];
        let mut out = Sexp::NULL;
        let t = capi::dotcode::robigo_dotcall(add3 as *const c_void, args.as_ptr(), 3, &mut out, std::ptr::null_mut());
        assert_eq!(t, 0);
        assert_eq!(*capi::memory::INTEGER(out), 42);
        assert_eq!(R_PPStackTop, 0, "arguments unprotected after the call");
    }
}
