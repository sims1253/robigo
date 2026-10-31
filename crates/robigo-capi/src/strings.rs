//! CHARSXPs and symbols: envir.c's mkCharLenCE, names.c's install,
//! sysutils.c's encoding functions (robigo runs in a UTF-8 locale).

use crate::errors::{error_str, error_str_owned};
use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Sexp, ce, gp};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};

/// EncodeString(c, 0, 0, Rprt_adj_none) for the embedded-nul message.
fn encode_for_message(b: &[u8]) -> String {
    let mut s = String::new();
    for &ch in b {
        match ch {
            0 => s.push_str("\\0"),
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            b'\\' => s.push_str("\\\\"),
            c if c < 0x20 || c == 0x7f => s.push_str(&format!("\\{:03o}", c)),
            c => s.push(c as char),
        }
    }
    s
}

pub fn enc_bits(enc: c_int) -> u16 {
    match enc {
        ce::UTF8 => gp::UTF8,
        ce::LATIN1 => gp::LATIN1,
        ce::BYTES => gp::BYTES,
        _ => 0,
    }
}

/// mkCharLenCE over a byte slice.
pub unsafe fn mk_char_bytes(b: &[u8], enc: c_int) -> Sexp {
    if !matches!(enc, ce::NATIVE | ce::UTF8 | ce::LATIN1 | ce::BYTES | ce::SYMBOL | ce::ANY) {
        error_str_owned(format!("unknown encoding: {enc}"))
    }
    if b.contains(&0) {
        error_str_owned(format!("embedded nul in string: '{}'", encode_for_message(b)))
    }
    core().mk_char(b, enc_bits(enc))
}

// R: envir.c:mkCharLenCE
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkCharLenCE(name: *const c_char, len: c_int, enc: c_int) -> Sexp {
    let b = if len > 0 { std::slice::from_raw_parts(name as *const u8, len as usize) } else { &[] };
    mk_char_bytes(b, enc)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkCharLen(name: *const c_char, len: c_int) -> Sexp {
    Rf_mkCharLenCE(name, len, ce::NATIVE)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkChar(name: *const c_char) -> Sexp {
    let b = CStr::from_ptr(name).to_bytes();
    core().mk_char(b, 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkCharCE(name: *const c_char, enc: c_int) -> Sexp {
    let b = CStr::from_ptr(name).to_bytes();
    mk_char_bytes(b, enc)
}

// R: Rinlinedfuns.h:mkString
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkString(s: *const c_char) -> Sexp {
    let t = core().alloc_vector(STRSXP, 1);
    Rf_protect(t);
    SET_STRING_ELT(t, 0, Rf_mkChar(s));
    Rf_unprotect(1);
    t
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ScalarString(x: Sexp) -> Sexp {
    Rf_protect(x);
    let t = core().alloc_vector(STRSXP, 1);
    SET_STRING_ELT(t, 0, x);
    Rf_unprotect(1);
    t
}

// R: sysutils.c:getCharCE
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_getCharCE(x: Sexp) -> c_int {
    if x.typ() != CHARSXP {
        error_str_owned(format!(
            "'{}' must be called on a CHARSXP, but got '{}'",
            "getCharCE",
            crate::util::type_to_char(x)
        ))
    }
    let g = x.gp();
    if g & gp::UTF8 != 0 {
        ce::UTF8
    } else if g & gp::LATIN1 != 0 {
        ce::LATIN1
    } else if g & gp::BYTES != 0 {
        ce::BYTES
    } else {
        ce::NATIVE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_charIsASCII(x: Sexp) -> c_int {
    (x.gp() & gp::ASCII != 0) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_charIsUTF8(x: Sexp) -> c_int {
    // R: sysutils.c — ASCII, UTF-8 marked, or native in a UTF-8 locale
    let g = x.gp();
    (g & (gp::ASCII | gp::UTF8) != 0 || g & (gp::LATIN1 | gp::BYTES) == 0) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_charIsLatin1(x: Sexp) -> c_int {
    let g = x.gp();
    (g & gp::ASCII != 0 || g & gp::LATIN1 != 0) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_ASCII(x: Sexp) -> c_int {
    Rf_charIsASCII(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_UTF8(x: Sexp) -> c_int {
    (x.gp() & gp::UTF8 != 0) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_LATIN1(x: Sexp) -> c_int {
    (x.gp() & gp::LATIN1 != 0) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_BYTES(x: Sexp) -> c_int {
    (x.gp() & gp::BYTES != 0) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ENC_KNOWN(x: Sexp) -> c_int {
    (x.gp() & (gp::UTF8 | gp::LATIN1) != 0) as c_int
}

/// A NUL-terminated copy in R_alloc memory.
unsafe fn r_alloc_str(b: &[u8]) -> *const c_char {
    let p = R_alloc(b.len() + 1, 1);
    std::ptr::copy_nonoverlapping(b.as_ptr(), p as *mut u8, b.len());
    *p.add(b.len()) = 0;
    p
}

fn latin1_to_utf8(b: &[u8]) -> Vec<u8> {
    b.iter().map(|&c| c as char).collect::<String>().into_bytes()
}

// R: sysutils.c:translateChar — the native encoding is UTF-8
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_translateChar(x: Sexp) -> *const c_char {
    if x.typ() != CHARSXP {
        error_str_owned(format!(
            "'{}' must be called on a CHARSXP, but got '{}'",
            "translateChar",
            crate::util::type_to_char(x)
        ))
    }
    let g = x.gp();
    if g & gp::LATIN1 != 0 {
        return r_alloc_str(&latin1_to_utf8(x.char_bytes()));
    }
    if g & gp::BYTES != 0 {
        error_str("translating strings with \"bytes\" encoding is not allowed")
    }
    x.std_data() as *const c_char
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_translateCharUTF8(x: Sexp) -> *const c_char {
    if x.typ() != CHARSXP {
        error_str_owned(format!(
            "'{}' must be called on a CHARSXP, but got '{}'",
            "translateCharUTF8",
            crate::util::type_to_char(x)
        ))
    }
    Rf_translateChar(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_translateChar0(x: Sexp) -> *const c_char {
    if x.gp() & gp::BYTES != 0 {
        return x.std_data() as *const c_char;
    }
    Rf_translateChar(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_translateCharFP(x: Sexp) -> *const c_char {
    Rf_translateChar(x)
}

// R: sysutils.c:reEnc, for the encodings robigo's UTF-8 locale meets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_reEnc(x: *const c_char, ce_in: c_int, ce_out: c_int, subst: c_int) -> *const c_char {
    let norm = |e: c_int| if e == ce::NATIVE { ce::UTF8 } else { e };
    let (ci, co) = (norm(ce_in), norm(ce_out));
    if ci == co
        || ci == ce::SYMBOL
        || co == ce::SYMBOL
        || ci == ce::ANY
        || co == ce::ANY
        || ci == ce::BYTES
        || co == ce::BYTES
    {
        return x;
    }
    let b = CStr::from_ptr(x).to_bytes();
    if b.is_ascii() {
        return x;
    }
    match (ci, co) {
        (ce::LATIN1, ce::UTF8) => r_alloc_str(&latin1_to_utf8(b)),
        (ce::UTF8, ce::LATIN1) => {
            let s = String::from_utf8_lossy(b);
            let mut out = Vec::new();
            for ch in s.chars() {
                let u = ch as u32;
                if u < 256 {
                    out.push(u as u8);
                } else {
                    let mut tmp = [0u8; 4];
                    for &byte in ch.encode_utf8(&mut tmp).as_bytes() {
                        match subst {
                            1 => out.extend_from_slice(format!("<{:02x}>", byte).as_bytes()),
                            2 => out.push(b'.'),
                            3 => {
                                out.extend_from_slice(format!("<U+{:04X}>", u).as_bytes());
                                break;
                            }
                            _ => out.push(b'?'),
                        }
                    }
                }
            }
            r_alloc_str(&out)
        }
        _ => x,
    }
}

// R: sysutils.c:ucstoutf8
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ucstoutf8(s: *mut c_char, c: c_uint) -> usize {
    if c == 0 {
        *s = 0;
        return 1;
    }
    let Some(ch) = char::from_u32(c) else {
        crate::errors::error_str_owned(format!("invalid Unicode point {c}"));
    };
    let mut buf = [0u8; 4];
    let n = ch.encode_utf8(&mut buf).len();
    std::ptr::copy_nonoverlapping(buf.as_ptr(), s as *mut u8, n);
    *s.add(n) = 0;
    n
}

fn utf8_valid(b: &[u8]) -> bool {
    std::str::from_utf8(b).is_ok()
}

// R: character.c:R_nchar (type: 0 bytes, 1 chars, 2 width)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_nchar(
    string: Sexp,
    type_: c_int,
    allow_na: c_int,
    keep_na: c_int,
    msg_name: *const c_char,
) -> c_int {
    if string == R_NaString {
        return if keep_na != 0 { i32::MIN } else { 2 };
    }
    let b = string.char_bytes();
    let g = string.gp();
    match type_ {
        0 => LENGTH(string),
        _ => {
            if g & gp::LATIN1 != 0 {
                return b.len() as c_int;
            }
            if g & gp::BYTES != 0 {
                if allow_na == 0 {
                    if !msg_name.is_null() {
                        let m = CStr::from_ptr(msg_name).to_string_lossy().into_owned();
                        error_str_owned(format!("number of characters is not computable in \"bytes\" encoding, {m}"))
                    }
                    return -2;
                }
                return i32::MIN;
            }
            if !utf8_valid(b) {
                if allow_na == 0 {
                    if !msg_name.is_null() {
                        let m = CStr::from_ptr(msg_name).to_string_lossy().into_owned();
                        error_str_owned(format!("invalid multibyte string, {m}"))
                    }
                    return -1;
                }
                return i32::MIN;
            }
            let s = std::str::from_utf8_unchecked(b);
            if type_ == 1 { s.chars().count() as c_int } else { robigo_runtime::format::str_width(s) as c_int }
        }
    }
}

// ---- symbols (names.c) --------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_install(name: *const c_char) -> Sexp {
    let b = CStr::from_ptr(name).to_bytes();
    if b.is_empty() {
        error_str("attempt to use zero-length variable name")
    }
    if b.len() > 10000 {
        error_str("variable names are limited to 10000 bytes")
    }
    core().install(b)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_installChar(x: Sexp) -> Sexp {
    if x.char_bytes().is_empty() {
        error_str("attempt to use zero-length variable name")
    }
    core().install(x.char_bytes())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_installTrChar(x: Sexp) -> Sexp {
    if x.typ() != CHARSXP {
        error_str_owned(format!(
            "'{}' must be called on a CHARSXP, but got '{}'",
            "installTrChar",
            crate::util::type_to_char(x)
        ))
    }
    let p = Rf_translateChar(x);
    let b = CStr::from_ptr(p).to_bytes();
    if b.is_empty() {
        error_str("attempt to use zero-length variable name")
    }
    core().install(b)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_installNoTrChar(x: Sexp) -> Sexp {
    Rf_installChar(x)
}

// ---- iconv (sysutils.c Riconv*) -------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Riconv_open(to: *const c_char, from: *const c_char) -> *mut c_void {
    let fix = |s: *const c_char| -> std::ffi::CString {
        let b = CStr::from_ptr(s).to_bytes();
        std::ffi::CString::new(if b.is_empty() { b"UTF-8".to_vec() } else { b.to_vec() }).unwrap()
    };
    let (t, f) = (fix(to), fix(from));
    libc::iconv_open(t.as_ptr(), f.as_ptr()) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Riconv(
    cd: *mut c_void,
    inbuf: *mut *const c_char,
    inbytesleft: *mut usize,
    outbuf: *mut *mut c_char,
    outbytesleft: *mut usize,
) -> usize {
    libc::iconv(cd as libc::iconv_t, inbuf as *mut *mut c_char, inbytesleft, outbuf, outbytesleft)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Riconv_close(cd: *mut c_void) -> c_int {
    libc::iconv_close(cd as libc::iconv_t)
}
