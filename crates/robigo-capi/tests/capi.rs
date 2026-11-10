#![allow(clippy::undocumented_unsafe_blocks)] // every call here is the C API itself
#![allow(static_mut_refs)] // assert_eq! on the exported statics
//! libR.so's API driven from Rust with a stub host. The API state is bound
//! to one thread (docs/core.md §3, §10), so everything runs in one test.

use R as capi;
use capi::Sexp;
use capi::state::*;
use robigo_core::types::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};

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
unsafe extern "C" fn h_op(name: *const c_char, a: *const Sexp, n_args: c_int, o: *mut Sexp) -> usize {
    let n = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    let args = unsafe { std::slice::from_raw_parts(a, n_args.max(0) as usize) };
    match n.as_str() {
        "toplevelBegin" | "toplevelEnd" => {
            unsafe { *o = R_NilValue };
            0
        }
        "newEnv" => {
            unsafe { *o = capi::memory::Rf_allocSExp(ENVSXP as c_uint) };
            0
        }
        // connection 3: "file" on "data.csv", open for reading
        "connInfo" => {
            unsafe {
                let info = capi::memory::Rf_protect(capi::memory::Rf_allocVector(VECSXP as u32, 4));
                for (k, s) in [c"file", c"data.csv", c"r"].iter().enumerate() {
                    capi::memory::SET_VECTOR_ELT(info, k as isize, capi::strings::Rf_mkString(s.as_ptr()));
                }
                capi::memory::SET_VECTOR_ELT(info, 3, capi::memory::Rf_allocVector(LGLSXP as u32, 7));
                std::ptr::write_bytes(capi::memory::LOGICAL(capi::memory::VECTOR_ELT(info, 3)), 0, 7);
                capi::memory::Rf_unprotect(1);
                *o = info;
            }
            0
        }
        "findVarInFrame" if args.len() == 2 => {
            let sym = args[1];
            let x_sym = unsafe { capi::strings::Rf_install(c"x".as_ptr()) };
            unsafe {
                *o = if sym == x_sym { capi::list::Rf_ScalarInteger(99) } else { R_UnboundValue };
            }
            0
        }
        "internal" if !args.is_empty() => {
            let op_name = unsafe { CStr::from_ptr(capi::memory::R_CHAR(capi::memory::STRING_ELT(args[0], 0))) }
                .to_string_lossy()
                .into_owned();
            match op_name.as_str() {
                "lockEnvironment" => {
                    log("locked env".to_string());
                    unsafe { *o = R_NilValue };
                    0
                }
                "unserialize" if args.len() >= 2 => {
                    unsafe { *o = args[1] };
                    0
                }
                _ => new_token(format!("op internal:{op_name}")),
            }
        }
        _ => new_token(format!("op {n}")),
    }
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
    ctx_save: h_ctx_save,
    ctx_restore: h_ctx_noop,
    ctx_drop: h_ctx_noop,
};

unsafe extern "C" fn h_ctx_save(out: *mut usize) {
    unsafe {
        *out = 0;
        *out.add(1) = 0;
    }
}
unsafe extern "C" fn h_ctx_noop(_: *const usize) {}

unsafe extern "C" fn body_error(_: *mut c_void) {
    unsafe { capi::errors::Rf_error(c"boom %d".as_ptr(), 42) }
}

unsafe extern "C" fn body_envlocked_null(_: *mut c_void) {
    unsafe {
        capi::envir::R_EnvironmentIsLocked(R_NilValue);
    }
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

unsafe extern "C" fn unwind_protect_error(_: *mut c_void) {
    unsafe {
        let cont = capi::memory::Rf_protect(capi::errors::R_MakeUnwindCont());
        capi::errors::R_UnwindProtect(fun_error, std::ptr::null_mut(), cleanup, std::ptr::null_mut(), cont);
        log("not reached: R_UnwindProtect continues the unwind".to_string());
    }
}

/// R_UnwindProtect whose body errors, inside R_ToplevelExec: the cleanup
/// runs with jump = 1, then the error continues to the outer context.
#[inline(never)]
unsafe fn unwind_and_abandon() -> usize {
    unsafe {
        let top = R_PPStackTop;
        assert_eq!(capi::errors::R_ToplevelExec(unwind_protect_error, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["cleanup jump=1".to_string(), "reported: in the body".to_string()]);
        assert_eq!(R_PPStackTop, top, "the body's PROTECTs were undone");
        pending_errors()
    }
}

unsafe extern "C" fn translate_bytes_utf8(_: *mut c_void) {
    unsafe {
        let b = capi::strings::Rf_mkCharCE(c"a\xffb".as_ptr(), 3);
        capi::strings::Rf_translateCharUTF8(b);
    }
}
unsafe extern "C" fn translate_bytes_native(_: *mut c_void) {
    unsafe {
        let b = capi::strings::Rf_mkCharCE(c"a\xffb".as_ptr(), 3);
        capi::strings::Rf_translateChar(b);
    }
}
unsafe extern "C" fn get_char_ce_of_int(_: *mut c_void) {
    unsafe {
        capi::strings::Rf_getCharCE(capi::list::Rf_ScalarInteger(1));
    }
}
unsafe extern "C" fn mk_char_nul(_: *mut c_void) {
    unsafe {
        capi::strings::Rf_mkCharLenCE(c"a".as_ptr(), 2, 1);
    }
}
unsafe extern "C" fn mk_char_symbol_new(_: *mut c_void) {
    unsafe {
        capi::strings::Rf_mkCharCE(c"\xce\xb1 never cached".as_ptr(), 5);
    }
}

/// CHARSXP encodings: cache identity per encoding, the ASCII mark, what
/// translation does in the UTF-8 locale robigo assumes, and its errors.
unsafe fn encodings() {
    unsafe {
        use capi::strings::*;
        let e = c"caf\xc3\xa9";
        let x: Vec<Sexp> = (0..4).map(|enc| capi::memory::Rf_protect(Rf_mkCharCE(e.as_ptr(), enc))).collect();
        for (i, a) in x.iter().enumerate() {
            assert_eq!(Rf_getCharCE(*a), i as c_int, "getCharCE reads the mark");
            assert_eq!(Rf_mkCharCE(e.as_ptr(), i as c_int), *a, "cached per (bytes, encoding)");
            assert_eq!(Rf_charIsASCII(*a), 0);
            for b in &x[i + 1..] {
                assert_ne!(a, b, "one CHARSXP for two encodings");
            }
        }
        assert_eq!(Rf_mkChar(e.as_ptr()), x[0], "mkChar is CE_NATIVE");
        for enc in [0, 1, 2, 3, 5, 99] {
            let a = Rf_mkCharCE(c"plain".as_ptr(), enc);
            assert_eq!(a, Rf_mkChar(c"plain".as_ptr()), "ASCII text is native in encoding {enc}");
            assert_eq!((Rf_charIsASCII(a), Rf_getCharCE(a)), (1, 0));
        }
        // the UTF-8 locale: native and UTF-8 text are used as they are
        assert_eq!(Rf_translateChar(x[0]), capi::memory::R_CHAR(x[0]));
        assert_eq!(Rf_translateCharUTF8(x[1]), capi::memory::R_CHAR(x[1]));
        assert_eq!((Rf_charIsUTF8(x[0]), Rf_charIsLatin1(x[0])), (1, 0));
        // latin1 is read as Windows-1252, and what has no character is escaped
        let l = Rf_mkCharCE(c"caf\xe9 \x80\x81".as_ptr(), 2);
        assert_eq!(CStr::from_ptr(Rf_translateCharUTF8(l)).to_bytes(), "café €<81>".as_bytes());
        assert_eq!(
            CStr::from_ptr(Rf_reEnc(c"A\xe2\x82\xacB".as_ptr(), 1, 2, 1)).to_bytes(),
            b"A<e2><82><ac>B",
            "reEnc substitutes byte by byte"
        );
        let mut buf = [0 as c_char; 8];
        assert_eq!(Rf_ucstoutf8(buf.as_mut_ptr(), 0x20ac), 3);
        assert_eq!(CStr::from_ptr(buf.as_ptr()).to_bytes(), "€".as_bytes());
        capi::memory::Rf_unprotect(4);
        for (body, msg) in [
            (
                translate_bytes_utf8 as unsafe extern "C" fn(*mut c_void),
                r#"translating strings with "bytes" encoding is not allowed"#,
            ),
            (translate_bytes_native, r#"translating strings with "bytes" encoding is not allowed"#),
            (get_char_ce_of_int, "'Rf_getCharCE' must be called on a CHARSXP, but got 'integer'"),
            (mk_char_nul, "embedded nul in string: 'a\\0'"),
            (mk_char_symbol_new, "unknown encoding mask: 5"),
        ] {
            assert_eq!(capi::errors::R_ToplevelExec(body, std::ptr::null_mut()), 0);
            assert_eq!(take_log(), vec![format!("reported: {msg}")]);
        }
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
        capi::host::robigo_gc();
        assert_eq!(pending_errors(), pending, "the continued exit was consumed, nothing left pending");

        let cont = capi::memory::Rf_protect(capi::errors::R_MakeUnwindCont());
        let r = capi::errors::R_UnwindProtect(fun_ok, std::ptr::null_mut(), cleanup, std::ptr::null_mut(), cont);
        assert_eq!(*capi::memory::INTEGER(r), 41);
        assert_eq!(take_log(), vec!["cleanup jump=0".to_string()]);
        capi::memory::Rf_unprotect(1);

        // ---- R_UnwindProtect with a NULL continuation (context.c:954): the
        // simple usage, which still continues the unwind after a jump
        unsafe extern "C" fn null_cont_jump(_: *mut c_void) {
            unsafe {
                capi::errors::R_UnwindProtect(
                    fun_error,
                    std::ptr::null_mut(),
                    cleanup,
                    std::ptr::null_mut(),
                    Sexp::NULL,
                );
                log("not reached: R_UnwindProtect continues the unwind".to_string());
            }
        }
        assert_eq!(capi::errors::R_ToplevelExec(null_cont_jump, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["cleanup jump=1".to_string(), "reported: in the body".to_string()]);
        let r = capi::errors::R_UnwindProtect(fun_ok, std::ptr::null_mut(), cleanup, std::ptr::null_mut(), Sexp::NULL);
        assert_eq!(*capi::memory::INTEGER(r), 41);
        assert_eq!(take_log(), vec!["cleanup jump=0".to_string()]);

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

        // ---- allocation failure is GNU R's error, not an abort
        unsafe extern "C" fn huge(_: *mut c_void) {
            unsafe { capi::memory::Rf_allocVector(REALSXP as u32, 1_000_000_000_000_000) };
        }
        assert_eq!(capi::errors::R_ToplevelExec(huge, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: cannot allocate vector of size 7450580.6 Gb".to_string()]);

        // R: memory.c:R_alloc goes through allocVector; 1e15 bytes do not fit
        // any address space, whatever the machine's overcommit policy
        unsafe extern "C" fn huge_r_alloc(_: *mut c_void) {
            unsafe { capi::memory::R_alloc(1_000_000_000_000_000, 1) };
        }
        assert_eq!(capi::errors::R_ToplevelExec(huge_r_alloc, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: cannot allocate vector of size 931322.6 Gb".to_string()]);

        // ---- memory.c:2755: only the cache makes CHARSXPs
        unsafe extern "C" fn charsxp_vector(_: *mut c_void) {
            unsafe { capi::memory::Rf_allocVector(CHARSXP as u32, 3) };
        }
        assert_eq!(capi::errors::R_ToplevelExec(charsxp_vector, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: use of allocVector(CHARSXP ...) is defunct\n".to_string()]);

        // ---- a CHARSXP's attribute slot is its hash: it stays in the cache
        let ch = capi::memory::Rf_protect(capi::strings::Rf_mkChar(c"attrib-free".as_ptr()));
        capi::memory::CLEAR_ATTRIB(ch);
        capi::memory::SET_ATTRIB(ch, R_NilValue);
        assert_eq!(capi::memory::ATTRIB(ch), R_NilValue);
        assert_eq!(capi::strings::Rf_mkChar(c"attrib-free".as_ptr()), ch, "the hash survived");
        unsafe extern "C" fn attrib_on_charsxp(_: *mut c_void) {
            unsafe {
                let ch = capi::memory::Rf_protect(capi::strings::Rf_mkChar(c"attrib-free".as_ptr()));
                let v = capi::memory::Rf_cons(R_NilValue, R_NilValue);
                capi::memory::SET_ATTRIB(ch, v);
            }
        }
        assert_eq!(capi::errors::R_ToplevelExec(attrib_on_charsxp, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: cannot set attribute on a CHARSXP".to_string()]);
        capi::host::robigo_gc();
        assert_eq!(capi::strings::Rf_mkChar(c"attrib-free".as_ptr()), ch, "the hash survived");
        capi::memory::Rf_unprotect(1);

        // ---- extents whose product overflows R_xlen_t
        unsafe extern "C" fn huge_3d(_: *mut c_void) {
            unsafe { capi::memory::Rf_alloc3DArray(INTSXP as u32, c_int::MAX, c_int::MAX, c_int::MAX) };
        }
        assert_eq!(capi::errors::R_ToplevelExec(huge_3d, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: 'alloc3DArray': too many elements specified".to_string()]);
        unsafe extern "C" fn huge_array(_: *mut c_void) {
            unsafe {
                let dims = capi::memory::Rf_protect(capi::memory::Rf_allocVector(INTSXP as u32, 3));
                for k in 0..3 {
                    *capi::memory::INTEGER(dims).add(k) = c_int::MAX;
                }
                capi::memory::Rf_allocArray(INTSXP as u32, dims);
            }
        }
        assert_eq!(capi::errors::R_ToplevelExec(huge_array, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: 'allocArray': too many elements specified by 'dims'".to_string()]);

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

        // ---- IS_SCALAR, XTRUELENGTH, LOGICAL_NO_NA, LOGICAL_IS_SORTED
        let s_int = capi::memory::Rf_protect(capi::list::Rf_ScalarInteger(7));
        assert_eq!(capi::util::IS_SCALAR(s_int, INTSXP as c_int), 1);
        assert_eq!(capi::util::IS_SCALAR(s_int, REALSXP as c_int), 0);
        assert_eq!(capi::util::IS_SCALAR(R_NilValue, INTSXP as c_int), 0);
        capi::memory::SET_TRUELENGTH(s_int, 19);
        assert_eq!(capi::util::XTRUELENGTH(s_int), 19);
        let s_lgl = capi::memory::Rf_protect(capi::list::Rf_ScalarLogical(1));
        assert_eq!(capi::altrep::LOGICAL_NO_NA(s_lgl), 0);
        assert_eq!(capi::altrep::LOGICAL_IS_SORTED(s_lgl), i32::MIN);
        capi::memory::Rf_unprotect(2);

        // ---- memory.c's CHK2 and Defn.h's SET_TRUELENGTH: neither a pairlist
        // nor an ALTREP object has a truelength slot to write
        unsafe extern "C" fn truelength_of_altrep(_: *mut c_void) {
            unsafe {
                let seq = capi::memory::Rf_protect(capi::compact::new_compact_intseq(10, 1, 1));
                capi::memory::SET_TRUELENGTH(seq, 3);
            }
        }
        assert_eq!(capi::errors::R_ToplevelExec(truelength_of_altrep, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: can't set ALTREP truelength".to_string()]);
        unsafe extern "C" fn truelength_of_pairlist(_: *mut c_void) {
            unsafe {
                let p = capi::memory::Rf_protect(capi::memory::Rf_cons(R_NilValue, R_NilValue));
                capi::memory::SET_TRUELENGTH(p, 3);
            }
        }
        assert_eq!(capi::errors::R_ToplevelExec(truelength_of_pairlist, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: LENGTH or similar applied to pairlist object".to_string()]);
        unsafe extern "C" fn truelength_of_null(_: *mut c_void) {
            unsafe { capi::memory::SET_TRUELENGTH(R_NilValue, 3) };
        }
        assert_eq!(capi::errors::R_ToplevelExec(truelength_of_null, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: LENGTH or similar applied to NULL object".to_string()]);
        assert_eq!(capi::memory::CDR(R_NilValue), R_NilValue, "R_NilValue is intact");
        let p = capi::memory::Rf_protect(capi::memory::Rf_cons(R_NilValue, R_NilValue));
        assert_eq!(capi::util::IS_SCALAR(p, LISTSXP as c_int), 0, "a pairlist is no scalar");
        assert_eq!(capi::memory::Rf_length(p), 1);
        capi::memory::Rf_unprotect(1);

        // ---- a host .Internal with any number of arguments (R_orderVector)
        unsafe extern "C" fn many_arguments(_: *mut c_void) {
            unsafe { capi::sort::internal("many", &[R_NilValue; 20]) };
        }
        assert_eq!(capi::errors::R_ToplevelExec(many_arguments, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: op internal:many".to_string()]);

        // ---- R_qsort, R_qsort_int, R_qsort_I, R_qsort_int_I, Rf_cPsort
        let mut rv = [99.0, 4.0, 1.5, 3.0, 2.0, -1.0, 88.0];
        capi::sort::R_qsort(rv.as_mut_ptr(), 2, 6);
        assert_eq!(rv, [99.0, -1.0, 1.5, 2.0, 3.0, 4.0, 88.0]);

        let mut iv: Vec<c_int> = (0..25).rev().collect();
        capi::sort::R_qsort_int(iv.as_mut_ptr(), 1, 25);
        let expected_iv: Vec<c_int> = (0..25).collect();
        assert_eq!(iv, expected_iv);

        let mut rv_i = [30.0, 10.0, 40.0, 20.0];
        let mut r_idx = [1, 2, 3, 4];
        capi::sort::R_qsort_I(rv_i.as_mut_ptr(), r_idx.as_mut_ptr(), 1, 4);
        assert_eq!(rv_i, [10.0, 20.0, 30.0, 40.0]);
        assert_eq!(r_idx, [2, 4, 1, 3]);

        let mut iv_i: Vec<c_int> = (1..=16).rev().collect();
        let mut i_idx: Vec<c_int> = (1..=16).collect();
        capi::sort::R_qsort_int_I(iv_i.as_mut_ptr(), i_idx.as_mut_ptr(), 1, 16);
        assert_eq!(iv_i, (1..=16).collect::<Vec<c_int>>());
        assert_eq!(i_idx, (1..=16).rev().collect::<Vec<c_int>>());

        let mut cv = [
            robigo_core::Rcomplex { r: 3.0, i: 1.0 },
            robigo_core::Rcomplex { r: 1.0, i: 2.0 },
            robigo_core::Rcomplex { r: 2.0, i: 0.0 },
        ];
        capi::sort::Rf_cPsort(cv.as_mut_ptr(), 3, 1);
        assert_eq!((cv[1].r, cv[1].i), (2.0, 0.0));

        // ---- R_tryEval vs R_tryEvalSilent
        let mut err = 0;
        assert_eq!(capi::envir::R_tryEval(R_NilValue, R_GlobalEnv, &mut err), Sexp::NULL);
        assert_eq!(err, 1);
        assert_eq!(take_log(), vec!["reported: eval not available".to_string()]);
        err = 0;
        assert_eq!(capi::envir::R_tryEvalSilent(R_NilValue, R_GlobalEnv, &mut err), Sexp::NULL);
        assert_eq!(err, 1);
        assert!(take_log().is_empty(), "R_tryEvalSilent discards the error without reporting");

        // ---- R_LockEnvironment, R_EnvironmentIsLocked, R_PackageEnvName, R_IsPackageEnv
        capi::envir::R_LockEnvironment(R_GlobalEnv, 1);
        assert_eq!(take_log(), vec!["locked env".to_string()]);
        // NULL is R's defunct error; the locked/unlocked and S4 paths run
        // for real in the capi corpus (cases/envir.R).
        assert_eq!(capi::errors::R_ToplevelExec(body_envlocked_null, std::ptr::null_mut()), 0);
        assert_eq!(take_log(), vec!["reported: use of NULL environment is defunct".to_string()]);
        // Only environments with a "package:" name are package envs; the
        // real thing is exercised by the capi corpus (cases/envir.R).
        assert_eq!(capi::envir::R_PackageEnvName(R_NilValue), R_NilValue);
        assert_eq!(capi::envir::R_IsPackageEnv(R_NilValue), 0);
        let not_env = capi::memory::Rf_protect(capi::list::Rf_ScalarInteger(1));
        assert_eq!(capi::envir::R_PackageEnvName(not_env), R_NilValue);
        assert_eq!(capi::envir::R_IsPackageEnv(not_env), 0);
        capi::memory::Rf_unprotect(1);

        // ---- Rf_substitute
        let plus = capi::strings::Rf_install(c"+".as_ptr());
        let x_sym = capi::strings::Rf_install(c"x".as_ptr());
        let y_sym = capi::strings::Rf_install(c"y".as_ptr());
        let lang = capi::memory::Rf_protect(capi::list::Rf_lang3(plus, x_sym, y_sym));
        let env = capi::envir::Rf_NewEnvironment(R_NilValue, R_NilValue, R_BaseEnv);
        let sub_env = capi::memory::Rf_protect(capi::envir::Rf_substitute(lang, env));
        assert_eq!(capi::memory::CAR(sub_env), plus);
        assert_eq!(*capi::memory::INTEGER(capi::memory::CADR(sub_env)), 99);
        assert_eq!(capi::memory::CADDR(sub_env), y_sym);
        // R_GlobalEnv is not substituted for a non-promise binding; in this
        // stub it is R_NilValue, which is the same "leave it alone" path.
        let sub_glob = capi::memory::Rf_protect(capi::envir::Rf_substitute(lang, R_GlobalEnv));
        assert_eq!(capi::memory::CADR(sub_glob), x_sym, "R_GlobalEnv does not substitute non-promise bindings");
        capi::memory::Rf_unprotect(3);

        // ---- R_InitInPStream and R_Unserialize
        struct BufSlice<'a> {
            bytes: &'a [u8],
            pos: usize,
        }
        unsafe extern "C" fn in_bytes(st: *mut capi::serialize::InPStream, dst: *mut c_void, n: c_int) {
            unsafe {
                let s = &mut *((*st).data as *mut BufSlice<'_>);
                let len = n as usize;
                std::ptr::copy_nonoverlapping(s.bytes.as_ptr().add(s.pos), dst as *mut u8, len);
                s.pos += len;
            }
        }
        // Minimal XDR version 2 stream for ScalarInteger(42), followed by trailing sentinel bytes
        let mut stream_bytes = Vec::new();
        stream_bytes.extend_from_slice(b"X\n");
        for word in [2i32, 0x040500, 0x030500, 13, 1, 42] {
            stream_bytes.extend_from_slice(&word.to_be_bytes());
        }
        let exact_len = stream_bytes.len();
        stream_bytes.extend_from_slice(b"TRAILING_UNREAD");
        let mut src = BufSlice { bytes: &stream_bytes, pos: 0 };
        let mut inp: capi::serialize::InPStream = std::mem::zeroed();
        capi::serialize::R_InitInPStream(
            &mut inp,
            &mut src as *mut BufSlice<'_> as *mut c_void,
            0,
            None,
            Some(in_bytes),
            None,
            R_NilValue,
        );
        let raw_read = capi::memory::Rf_protect(capi::serialize::R_Unserialize(&mut inp));
        assert_eq!(src.pos, exact_len, "R_Unserialize stops at the exact end of the serialized item");
        assert_eq!(capi::memory::LENGTH(raw_read) as usize, exact_len);
        capi::memory::Rf_unprotect(1);

        // ---- a connection's strings stay where C saw them
        let con = capi::connections::getConnection(3);
        let (class, description) = ((*con).class, (*con).description);
        assert_eq!(CStr::from_ptr(description).to_bytes(), b"data.csv");
        assert_eq!(capi::connections::getConnection(3), con);
        assert_eq!((*con).class, class);
        assert_eq!((*con).description, description, "a saved con->description stays valid");

        // ---- corrupt lengths are R errors: an RAWSXP of 2^47 bytes from the
        // long-vector form, and a length past what ReadLENGTH accepts
        thread_local! {
            static CORRUPT: RefCell<(Vec<u8>, usize)> = const { RefCell::new((Vec::new(), 0)) };
        }
        unsafe extern "C" fn in_corrupt(_: *mut capi::serialize::InPStream, dst: *mut c_void, n: c_int) {
            assert!(n >= 0, "in_bytes got a negative length");
            CORRUPT.with_borrow_mut(|(data, pos)| {
                let len = (n as usize).min(data.len() - *pos);
                unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().add(*pos), dst as *mut u8, len) };
                *pos += len;
            });
        }
        unsafe extern "C" fn read_corrupt(_: *mut c_void) {
            unsafe {
                let mut inp: capi::serialize::InPStream = std::mem::zeroed();
                capi::serialize::R_InitInPStream(
                    &mut inp,
                    std::ptr::null_mut(),
                    0,
                    None,
                    Some(in_corrupt),
                    None,
                    R_NilValue,
                );
                capi::serialize::R_Unserialize(&mut inp);
            }
        }
        let corrupt = |hi: i32, lo: u32, ty: i32| {
            let mut bytes = b"X\n".to_vec();
            for word in [2, 0x040500, 0x030500, ty, -1, hi, lo as i32] {
                bytes.extend_from_slice(&i32::to_be_bytes(word));
            }
            CORRUPT.set((bytes, 0));
            capi::errors::R_ToplevelExec(read_corrupt, std::ptr::null_mut())
        };
        let top = R_PPStackTop;
        assert_eq!(corrupt(70000, 0, 24), 0);
        assert_eq!(take_log(), vec!["reported: invalid upper part of serialized vector length".to_string()]);
        assert_eq!(corrupt(32768, 0, 24), 0);
        assert_eq!(take_log(), vec!["reported: cannot allocate vector of size 131072.0 Gb".to_string()]);
        assert_eq!(R_PPStackTop, top, "the scanner's protection is undone");
        // a payload past INT_MAX arrives in chunks in_bytes can take (the
        // stub writes none of it, so this costs address space only)
        assert_eq!(corrupt(0, (1 << 31) + 10, 24), 1, "read through");
        assert!(take_log().is_empty());

        encodings();
    }
}

/// The offsets context.rs uses come from the C compiler; on this platform
/// they are the ones measured against GNU R's Defn.h.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn rcntxt_offsets_are_gnu_rs() {
    assert_eq!(R::context::rcntxt_offsets(), [8, 272, 280]);
}

/// R: unix/system.c:Rf_initialize_R. These are data packages read directly.
#[test]
fn console_data_is_initialized() {
    use std::ffi::CStr;
    unsafe extern "C" {
        static R_GUIType: *const std::ffi::c_char;
        static R_Outputfile: *mut libc::FILE;
        static R_Consolefile: *mut libc::FILE;
    }
    unsafe {
        capi::state::console_init();
        assert_eq!(CStr::from_ptr(R_GUIType).to_bytes(), b"X11");
        assert!(!R_Outputfile.is_null() && !R_Consolefile.is_null());
        assert_eq!(libc::fileno(R_Outputfile), 1);
        assert_eq!(libc::fileno(R_Consolefile), 2);
    }
}

/// `struct Rconn` as GNU R 4.5's R_ext/Connections.h lays it out (offsets
/// measured with gcc against the header, x86_64).
#[test]
fn rconn_layout_is_gnu_rs() {
    use R::connections::Rconn;
    use std::mem::{offset_of, size_of};
    assert_eq!(offset_of!(Rconn, open), 64);
    assert_eq!(offset_of!(Rconn, read), 136);
    assert_eq!(offset_of!(Rconn, n_push_back), 152);
    assert_eq!(offset_of!(Rconn, encname), 176);
    assert_eq!(offset_of!(Rconn, inconv), 280);
    assert_eq!(offset_of!(Rconn, navail), 410);
    assert_eq!(offset_of!(Rconn, id), 424);
    assert_eq!(offset_of!(Rconn, status), 448);
    assert_eq!(offset_of!(Rconn, buff_pos), 480);
    assert_eq!(size_of::<Rconn>(), 488);
}
