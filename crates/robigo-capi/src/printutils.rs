//! printutils.c/util.c/unique.c/connections.c entry points the base
//! packages' C code calls, over the runtime's implementations.

use std::ffi::{CString, c_char, c_int, c_void};

use crate::errors::{Res, ret};
use crate::list::{Rf_ScalarInteger, Rf_ScalarLogical};
use crate::memory::*;
use crate::state::*;
use robigo_core::Sexp;
use robigo_runtime::format::{Justify, PrintParams, encode_string};

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
    // EncodeString's result buffer (printutils.c keeps a static one), in
    // the native encoding
    let e = &crate::state::tables().encoded;
    *e.borrow_mut() = CString::new(robigo_runtime::locale::output_bytes(&out).into_owned()).unwrap_or_default();
    e.borrow().as_ptr()
}

// R: printutils.c:EncodeChar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_EncodeChar(x: Sexp) -> *const c_char {
    Rf_EncodeString(x, 0, 0, 0)
}

// R: util.c:Scollate — the collation order of two CHARSXPs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_Scollate(a: Sexp, b: Sexp) -> c_int {
    ret(scollate(a, b))
}

/// # Safety
/// `a` and `b` are CHARSXPs the C caller keeps live for the whole call, so
/// their bytes, which `x` and `y` borrow when they are valid UTF-8, stay
/// valid throughout; nothing here frees or moves them.
unsafe fn scollate(a: Sexp, b: Sexp) -> Res<c_int> {
    let (x, y) = (String::from_utf8_lossy(a.char_bytes()), String::from_utf8_lossy(b.char_bytes()));
    if x == y {
        return Ok(0);
    }
    // Setting up the collator on first use can fail as in GNU R, with an
    // error() against the context call.
    match robigo_runtime::collate::compare(&x, &y) {
        Ok(o) => Ok(o as c_int),
        Err(robigo_runtime::error::Flow::Error(e)) => Err(crate::errors::error_str(&e.message)),
        Err(_) => Err(crate::errors::error_str("robigo: unexpected non-local exit in Scollate")),
    }
}

// R: unique.c:matchE — match(ix, itable, nomatch)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_matchE(itable: Sexp, ix: Sexp, nmatch: c_int, _env: Sexp) -> Sexp {
    ret(match_e(itable, ix, nmatch))
}

unsafe fn match_e(itable: Sexp, ix: Sexp, nmatch: c_int) -> Res<Sexp> {
    let nm = protect(Rf_ScalarInteger(nmatch))?;
    let r = crate::sort::internal("match", &[ix, itable, nm, R_NilValue])?;
    unprotect(1)?;
    Ok(r)
}

// R: connections.c:con_pushback — through the connection's pushBack
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_con_pushback(con: *mut c_void, new_line: c_int, line: *const c_char) {
    ret(con_pushback(con, new_line, line))
}

unsafe fn con_pushback(con: *mut c_void, new_line: c_int, line: *const c_char) -> Res<()> {
    let n = crate::connections::number_of(con)?;
    let data = protect(crate::strings::mk_string(line)?)?;
    let c = protect(Rf_ScalarInteger(n))?;
    let nl = protect(Rf_ScalarLogical(new_line))?;
    let ty = protect(Rf_ScalarInteger(1))?;
    crate::sort::internal("pushBack", &[data, c, nl, ty])?;
    unprotect(4)
}

// R: errors.c:R_SignalCStackOverflow
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_SignalCStackOverflow(usage: isize) -> ! {
    crate::errors::throw(crate::memory::signal_cstack_overflow(usage))
}
