//! printutils.c/util.c/unique.c/connections.c entry points the base
//! packages' C code calls, over the runtime's implementations.

use std::cell::RefCell;
use std::ffi::{CString, c_char, c_int, c_void};

use crate::list::{Rf_ScalarInteger, Rf_ScalarLogical};
use crate::memory::*;
use crate::state::*;
use robigo_core::Sexp;
use robigo_runtime::format::{Justify, PrintParams, encode_string};

thread_local! {
    /// EncodeString's result buffer (printutils.c keeps a static one)
    static ENCODED: RefCell<CString> = RefCell::new(CString::default());
}

unsafe fn char_str(x: Sexp) -> robigo_runtime::value::RStr {
    if x == R_NaString { None } else { crate::bridge::rstr_of(x) }
}

// R: printutils.c:EncodeString — the CHARSXP escaped (and quoted), padded
// to width w; the result lives until the next call
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_EncodeString(s: Sexp, w: c_int, quote: c_int, justify: c_int) -> *const c_char {
    let j = match justify {
        0 => Justify::Left,
        1 => Justify::Right,
        2 => Justify::Centre,
        _ => Justify::None,
    };
    let q = if quote != 0 { Some(quote as u8 as char) } else { None };
    let pp = PrintParams { na_string: "NA".into(), na_string_noquote: "<NA>".into(), ..Default::default() };
    let out = encode_string(&char_str(s), w.max(0) as usize, q, j, &pp);
    ENCODED.with(|e| {
        *e.borrow_mut() = CString::new(out).unwrap_or_default();
        e.borrow().as_ptr()
    })
}

// R: printutils.c:EncodeChar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_EncodeChar(x: Sexp) -> *const c_char {
    Rf_EncodeString(x, 0, 0, 0)
}

// R: util.c:Scollate — the collation order of two CHARSXPs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_Scollate(a: Sexp, b: Sexp) -> c_int {
    let (x, y) = (String::from_utf8_lossy(a.char_bytes()), String::from_utf8_lossy(b.char_bytes()));
    robigo_runtime::builtins::options::scollate(&x, &y) as c_int
}

// R: unique.c:matchE — match(ix, itable, nomatch)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_matchE(itable: Sexp, ix: Sexp, nmatch: c_int, _env: Sexp) -> Sexp {
    let nm = Rf_protect(Rf_ScalarInteger(nmatch));
    let r = crate::sort::internal("match", &[ix, itable, nm, R_NilValue]);
    Rf_unprotect(1);
    r
}

// R: connections.c:con_pushback — through the connection's pushBack
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_con_pushback(con: *mut c_void, new_line: c_int, line: *const c_char) {
    let n = crate::connections::number_of(con);
    let data = Rf_protect(crate::strings::Rf_mkString(line));
    let c = Rf_protect(Rf_ScalarInteger(n));
    let nl = Rf_protect(Rf_ScalarLogical(new_line));
    let ty = Rf_protect(Rf_ScalarInteger(1));
    crate::sort::internal("pushBack", &[data, c, nl, ty]);
    Rf_unprotect(4);
}

// R: errors.c:R_SignalCStackOverflow
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_SignalCStackOverflow(usage: isize) -> ! {
    crate::memory::signal_cstack_overflow(usage)
}
