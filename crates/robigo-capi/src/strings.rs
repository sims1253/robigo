//! CHARSXPs and symbols: envir.c's mkCharLenCE, names.c's install,
//! sysutils.c's encoding functions and character.c's R_nchar. The
//! encoding rules are robigo-core's (`chars`); the conversions are
//! GNU R's, through iconv, in whatever locale platform.c's
//! R_check_locale (`check_locale`) last found.

use crate::errors::{Res, Unwind, error_str, ret, throw, warning_str};
use crate::memory::*;
use crate::state::*;
use robigo_core::chars::{self, BytesEncoding, Locale, MkCharError, Translation};
use robigo_core::types::*;
use robigo_core::{Sexp, ce, gp};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};

/// The locale as platform.c:R_check_locale last read it.
#[inline]
pub fn locale() -> Locale {
    // SAFETY: plain reads of C globals that only the R thread writes.
    unsafe { Locale { utf8: utf8locale != 0, latin1: latin1locale != 0 } }
}

/// platform.c:R_check_locale and invalidate_cached_recodings: re-read the
/// C library's LC_CTYPE into the locale globals and drop the cached
/// conversions (they convert from or to the old native encoding).
pub unsafe fn check_locale() {
    robigo_locale_init();
    ICONV_CACHE.with(|c| {
        for slot in c.iter() {
            let cd = slot.replace(None);
            if let Some(cd) = cd {
                libc::iconv_close(cd as libc::iconv_t);
            }
        }
    });
}

/// EncodeString(c, 0, 0, Rprt_adj_none) of the CHARSXP mkCharLenCE
/// builds for the embedded-nul message: `b` marked `enc` (ASCII text
/// unmarked). Its first steps are C-string ones, so a "bytes" string
/// becomes `\xNN` escapes up to the first NUL (then escaped again as
/// native text), a string that needs translation stops at the first NUL,
/// and only an untranslated one shows its NULs, as `\0`.
/// R: printutils.c:EncodeString
unsafe fn encode_for_message(b: &[u8], enc: c_int) -> Res<String> {
    use robigo_runtime::format::{Justify, PrintParams, encode_string};
    use robigo_runtime::value::{Encoding, RString};
    // R: the CHARSXP keeps enc's mark, and gets ASCII as well
    let g = chars::ce_bits(enc) | if b.is_ascii() { gp::ASCII } else { 0 };
    let upto_nul = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
    let text: Vec<u8> = if g & gp::BYTES != 0 {
        let mut t = Vec::new();
        for &k in upto_nul {
            if (0x20..0x80).contains(&k) {
                t.push(k);
            } else {
                t.extend_from_slice(format!("\\x{k:02x}").as_bytes());
            }
        }
        t
    } else {
        match chars::to_native(g, false, locale()) {
            Ok(Translation::None) | Err(_) => b.to_vec(),
            Ok(t) => translate_to_native(upto_nul, t)?.0,
        }
    };
    let pp = PrintParams { na_string: "NA".into(), na_string_noquote: "<NA>".into(), ..Default::default() };
    let parts: Vec<String> = text
        .split(|&c| c == 0)
        .map(|seg| encode_string(&Some(RString::from_bytes(seg, Encoding::Native)), 0, None, Justify::None, &pp))
        .collect();
    Ok(parts.join("\\0"))
}

/// mkCharLenCE over a byte slice.
// R: envir.c:mkCharLenCE
pub unsafe fn mk_char_bytes(b: &[u8], enc: c_int) -> Res<Sexp> {
    match core().mk_char_ce(b, enc) {
        Ok(c) => Ok(c),
        Err(MkCharError::UnknownEncoding(e)) => Err(error_str(&format!("unknown encoding: {e}"))),
        Err(MkCharError::EmbeddedNul) => {
            Err(error_str(&format!("embedded nul in string: '{}'", encode_for_message(b, enc)?)))
        }
        Err(MkCharError::UnknownMask(e)) => Err(error_str(&format!("unknown encoding mask: {e}"))),
    }
}

/// envir.c:mkCharCE's and mkChar's length check, made before the lookup
/// as GNU R's is.
#[inline(always)]
fn c_length_ok(b: &[u8]) -> bool {
    b.len() <= c_int::MAX as usize
}

#[cold]
unsafe fn c_length_error() -> Unwind {
    error_str("R character strings are limited to 2^31-1 bytes")
}

/// mkCharLenCE.
pub unsafe fn mk_char_len_ce(name: *const c_char, len: c_int, enc: c_int) -> Res<Sexp> {
    let b = if len > 0 { std::slice::from_raw_parts(name as *const u8, len as usize) } else { &[] };
    mk_char_bytes(b, enc)
}

// R: envir.c:mkCharLenCE
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkCharLenCE(name: *const c_char, len: c_int, enc: c_int) -> Sexp {
    ret(mk_char_len_ce(name, len, enc))
}

// R: envir.c:mkCharLen
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkCharLen(name: *const c_char, len: c_int) -> Sexp {
    ret(mk_char_len_ce(name, len, ce::NATIVE))
}

// R: envir.c:mkChar. A C string has no NUL inside and CE_NATIVE is valid,
// so this is mkCharLenCE without its two checks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkChar(name: *const c_char) -> Sexp {
    let b = CStr::from_ptr(name).to_bytes();
    if !c_length_ok(b) {
        throw(c_length_error())
    }
    core().mk_char(b, 0)
}

/// mkChar.
#[inline]
pub unsafe fn mk_char(name: *const c_char) -> Res<Sexp> {
    let b = CStr::from_ptr(name).to_bytes();
    if !c_length_ok(b) {
        return Err(c_length_error());
    }
    Ok(core().mk_char(b, 0))
}

/// mkCharCE.
pub unsafe fn mk_char_ce(name: *const c_char, enc: c_int) -> Res<Sexp> {
    let b = CStr::from_ptr(name).to_bytes();
    if !c_length_ok(b) {
        return Err(c_length_error());
    }
    mk_char_bytes(b, enc)
}

// R: envir.c:mkCharCE
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkCharCE(name: *const c_char, enc: c_int) -> Sexp {
    ret(mk_char_ce(name, enc))
}

// R: Rinlinedfuns.h:mkString
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkString(s: *const c_char) -> Sexp {
    ret(mk_string(s))
}

/// mkString.
pub unsafe fn mk_string(s: *const c_char) -> Res<Sexp> {
    let t = core().alloc_vector(STRSXP, 1);
    protect(t)?;
    set_string_elt(t, 0, mk_char(s)?)?;
    unprotect(1)?;
    Ok(t)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ScalarString(x: Sexp) -> Sexp {
    ret(scalar_string(x))
}

/// ScalarString.
pub unsafe fn scalar_string(x: Sexp) -> Res<Sexp> {
    protect(x)?;
    let t = core().alloc_vector(STRSXP, 1);
    set_string_elt(t, 0, x)?;
    unprotect(1)?;
    Ok(t)
}

/// sysutils.c's CHECK_CHARSXP; `func` is the C function's `__func__`,
/// which Rinternals.h's remapping makes the `Rf_` name.
#[inline(always)]
unsafe fn check_charsxp(x: Sexp, func: &str) -> Res<()> {
    if x.typ() != CHARSXP {
        return Err(not_charsxp(x, func));
    }
    Ok(())
}

#[cold]
#[inline(never)]
unsafe fn not_charsxp(x: Sexp, func: &str) -> Unwind {
    error_str(&format!("'{}' must be called on a CHARSXP, but got '{}'", func, crate::util::type_to_char(x)))
}

/// getCharCE.
pub unsafe fn get_char_ce(x: Sexp) -> Res<c_int> {
    check_charsxp(x, "Rf_getCharCE")?;
    Ok(chars::char_ce(x.gp()))
}

// R: sysutils.c:getCharCE
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_getCharCE(x: Sexp) -> c_int {
    ret(get_char_ce(x))
}

// R: sysutils.c:charIsASCII
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_charIsASCII(x: Sexp) -> c_int {
    ret(check_charsxp(x, "Rf_charIsASCII"));
    (x.gp() & gp::ASCII != 0) as c_int
}

// R: sysutils.c:charIsUTF8
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_charIsUTF8(x: Sexp) -> c_int {
    ret(check_charsxp(x, "Rf_charIsUTF8"));
    chars::is_utf8(x.gp(), x == R_NaString, locale()) as c_int
}

// R: sysutils.c:charIsLatin1
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_charIsLatin1(x: Sexp) -> c_int {
    ret(check_charsxp(x, "Rf_charIsLatin1"));
    chars::is_latin1(x.gp(), x == R_NaString, locale()) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_ASCII(x: Sexp) -> c_int {
    (x.gp() & gp::ASCII != 0) as c_int
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

/// A NUL-terminated copy in R_alloc memory (sysutils.c's
/// copyAndFreeStringBuffer).
unsafe fn r_alloc_str(b: &[u8]) -> Res<*const c_char> {
    let p = r_alloc(b.len() + 1, 1)?;
    std::ptr::copy_nonoverlapping(b.as_ptr(), p as *mut u8, b.len());
    *p.add(b.len()) = 0;
    Ok(p)
}

// ---- conversion through iconv (sysutils.c) -------------------------------------

/// GNU R's build has HAVE_ICONV_CP1252: "latin1" strings are read as
/// Windows-1252, so 0x80 is the euro sign.
const LATIN1_FROM: &CStr = c"CP1252";
#[cfg(target_endian = "little")]
const UNICODE: &CStr = c"UCS-4LE";
#[cfg(target_endian = "big")]
const UNICODE: &CStr = c"UCS-4BE";

/// The conversions sysutils.c keeps open: latin1_obj, utf8_obj (to the
/// native encoding) and ucsutf8_obj. check_locale closes them.
const FROM_LATIN1: usize = 0;
const FROM_UTF8: usize = 1;
const UCS_UTF8: usize = 2;

thread_local! {
    static ICONV_CACHE: [std::cell::Cell<Option<usize>>; 3] = const {
        [std::cell::Cell::new(None), std::cell::Cell::new(None), std::cell::Cell::new(None)]
    };
}

/// The cached conversion `slot`, opened with `to`/`from` on first use, or
/// GNU R's "unsupported conversion" error.
unsafe fn cached_iconv(slot: usize, to: &CStr, from: &CStr) -> Res<libc::iconv_t> {
    if let Some(cd) = ICONV_CACHE.with(|c| c[slot].get()) {
        return Ok(cd as libc::iconv_t);
    }
    let cd = Riconv_open(to.as_ptr(), from.as_ptr());
    if cd as isize == -1 {
        return Err(error_str(&format!(
            "unsupported conversion from '{}' to '{}'",
            from.to_string_lossy(),
            to.to_string_lossy()
        )));
    }
    ICONV_CACHE.with(|c| c[slot].set(Some(cd as usize)));
    Ok(cd as libc::iconv_t)
}

fn errno() -> c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Convert `input` with `cd`, the loop sysutils.c's translateTo* and
/// reEncodeIconv share. Where iconv stops at an invalid or incomplete
/// sequence (EILSEQ, EINVAL), it resets the shift state and `bad(rest,
/// out)` writes a substitute for the start of `rest` and returns how many
/// bytes it consumed (at least one).
unsafe fn convert(cd: libc::iconv_t, input: &[u8], mut bad: impl FnMut(&[u8], &mut Vec<u8>) -> usize) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(2 * input.len() + 16);
    let mut inp = input.as_ptr() as *mut c_char;
    let mut inb = input.len();
    // R: "First initialize output"
    libc::iconv(cd, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
    // one iconv call into the spare capacity: its result and errno
    let step = |out: &mut Vec<u8>, inp: *mut *mut c_char, inb: *mut usize| -> (usize, c_int) {
        if out.capacity() - out.len() < 16 {
            out.reserve(out.capacity().max(16));
        }
        let spare = out.capacity() - out.len();
        let mut op = out.as_mut_ptr().add(out.len()) as *mut c_char;
        let mut ob = spare;
        let r = libc::iconv(cd, inp, inb, &mut op, &mut ob);
        let e = errno();
        out.set_len(out.len() + (spare - ob));
        (r, e)
    };
    loop {
        let (r, e) = step(&mut out, &mut inp, &mut inb);
        if r != usize::MAX {
            break;
        }
        match e {
            libc::E2BIG => out.reserve(out.capacity()),
            libc::EILSEQ | libc::EINVAL => {
                // R: res = Riconv(obj, NULL, NULL, &outbuf, &outb)
                while let (usize::MAX, libc::E2BIG) = step(&mut out, std::ptr::null_mut(), std::ptr::null_mut()) {
                    out.reserve(out.capacity());
                }
                let rest = std::slice::from_raw_parts(inp as *const u8, inb);
                let n = bad(rest, &mut out).min(inb);
                inp = inp.add(n);
                inb -= n;
            }
            _ => break,
        }
    }
    out
}

/// `<xx>` for the byte iconv could not convert.
fn subst_hex(rest: &[u8], out: &mut Vec<u8>) -> usize {
    out.extend_from_slice(format!("<{:02x}>", rest[0]).as_bytes());
    1
}

unsafe extern "C" {
    // robigo_glue.c (GNU R's util.c and valid_utf8.h)
    fn Rf_utf8toucs(wc: *mut libc::wchar_t, s: *const c_char) -> usize;
    fn utf8clen(c: c_char) -> c_int;
    fn utf8Valid(s: *const c_char) -> c_int;
    // robigo_util.c (GNU R's util.c)
    fn mbcsTruncateToValid(s: *mut c_char) -> *mut c_char;
    // rlocale.c
    fn Ri18n_wcwidth(c: libc::wchar_t) -> c_int;
    fn Ri18n_wcswidth(s: *const libc::wchar_t, n: usize) -> c_int;
    // robigo_sys.c (GNU R's platform.c:R_check_locale)
    fn robigo_locale_init();
    // the C library (not in the libc crate)
    fn mbstowcs(dest: *mut libc::wchar_t, src: *const c_char, n: usize) -> usize;
}

/// The character at the start of `rest` as utf8toucs reads it: its length
/// in bytes (or `None` when it is invalid) and its code point.
fn utf8_char(rest: &[u8]) -> Option<(usize, u32)> {
    let mut buf = [0u8; 8];
    let n = rest.len().min(7);
    buf[..n].copy_from_slice(&rest[..n]);
    let mut wc: libc::wchar_t = 0;
    // SAFETY: `buf` is NUL-terminated, and utf8toucs reads up to the NUL.
    let clen = unsafe { Rf_utf8toucs(&mut wc, buf.as_ptr() as *const c_char) } as isize;
    if clen > 0 { Some((clen as usize, wc as u32)) } else { None }
}

/// GNU R's message argument: at most 255 bytes, else 252 of them cut back
/// to whole characters, then "...".
unsafe fn truncate_for_message(b: &[u8]) -> String {
    if b.len() > 255 {
        let mut buf = [0u8; 256];
        buf[..252].copy_from_slice(&b[..252]);
        mbcsTruncateToValid(buf.as_mut_ptr() as *mut c_char);
        let mut v = CStr::from_bytes_until_nul(&buf).map(|c| c.to_bytes().to_vec()).unwrap_or_default();
        v.extend_from_slice(b"...");
        String::from_utf8_lossy(&v).into_owned()
    } else {
        String::from_utf8_lossy(b).into_owned()
    }
}

/// The conversion of `ans` to the native encoding, and whether some of it
/// failed (and was escaped as `<U+xxxx>` or `<xx>`).
/// R: sysutils.c:translateToNative
unsafe fn translate_to_native(ans: &[u8], t: Translation) -> Res<(Vec<u8>, bool)> {
    let cd = match t {
        Translation::FromLatin1 => cached_iconv(FROM_LATIN1, c"", LATIN1_FROM)?,
        Translation::FromUtf8 => cached_iconv(FROM_UTF8, c"", c"UTF-8")?,
        _ => return Err(error_str("internal error: no translation needed")),
    };
    let mut failed = false;
    let out = convert(cd, ans, |rest, out| {
        failed = true;
        if t == Translation::FromUtf8 {
            // R: if starting in UTF-8, use \uxxxx
            if let Some((clen, ucs)) = utf8_char(rest).filter(|&(clen, _)| rest.len() >= clen) {
                let esc = if ucs < 65536 { format!("<U+{:04X}>", ucs) } else { format!("<U+{:08X}>", ucs) };
                out.extend_from_slice(esc.as_bytes());
                return clen;
            }
        }
        subst_hex(rest, out)
    });
    Ok((out, failed))
}

/// translateToNative's `mustWork`: a failed conversion is an error (1) or
/// a warning (2), with the escaped text; false after the warning.
unsafe fn must_work_native(out: &[u8], must_work: c_int) -> Res<bool> {
    let msg = || format!("unable to translate '{}' to native encoding", truncate_for_message(out));
    match must_work {
        1 => Err(error_str(&msg())),
        2 => {
            warning_str(&msg())?;
            Ok(false)
        }
        _ => Ok(true),
    }
}

/// The conversion of `ans` to UTF-8, as translate_to_native.
/// R: sysutils.c:translateToUTF8
unsafe fn translate_to_utf8(ans: &[u8], t: Translation, must_work: c_int) -> Res<Option<Vec<u8>>> {
    let from = match t {
        Translation::FromLatin1 => LATIN1_FROM,
        Translation::FromNative => c"",
        _ => return Err(error_str("internal error: no translation needed")),
    };
    // R: opened and closed for every string
    let cd = Riconv_open(c"UTF-8".as_ptr(), from.as_ptr());
    if cd as isize == -1 {
        return Err(error_str(&format!("unsupported conversion from '{}' to '{}'", from.to_string_lossy(), "UTF-8")));
    }
    let mut failed = false;
    let mut out = convert(cd as libc::iconv_t, ans, |rest, out| {
        failed = true;
        subst_hex(rest, out)
    });
    Riconv_close(cd);
    if must_work != 0 && failed {
        out.push(0);
        let native = CStr::from_ptr(re_enc(out.as_ptr() as *const c_char, ce::UTF8, ce::NATIVE, 2)?).to_bytes();
        let msg = format!("unable to translate '{}' to UTF-8", truncate_for_message(native));
        if must_work == 2 {
            warning_str(&msg)?;
            return Ok(None);
        }
        return Err(error_str(&msg));
    }
    Ok(Some(out))
}

/// What translating `x` to the native encoding needs, or GNU R's error
/// for a "bytes" string.
#[inline(always)]
unsafe fn needs_translation(x: Sexp) -> Res<Translation> {
    chars::to_native(x.gp(), x == R_NaString, locale()).map_err(|_| bytes_error())
}

#[inline(always)]
unsafe fn needs_translation_utf8(x: Sexp) -> Res<Translation> {
    chars::to_utf8(x.gp(), x == R_NaString, locale()).map_err(|_| bytes_error())
}

#[cold]
#[inline(never)]
unsafe fn bytes_error() -> Unwind {
    error_str(BytesEncoding::MESSAGE)
}

/// translateChar's work, for the C function `func` with translateToNative's
/// `mustWork`.
unsafe fn translate_char_as(x: Sexp, func: &str, must_work: c_int) -> Res<*const c_char> {
    check_charsxp(x, func)?;
    let t = needs_translation(x)?;
    if t == Translation::None {
        return Ok(x.std_data() as *const c_char);
    }
    let (v, failed) = translate_to_native(x.char_bytes(), t)?;
    if failed && !must_work_native(&v, must_work)? {
        return Ok(std::ptr::null());
    }
    r_alloc_str(&v)
}

/// translateChar.
pub unsafe fn translate_char(x: Sexp) -> Res<*const c_char> {
    translate_char_as(x, "Rf_translateChar", 0)
}

// R: sysutils.c:translateChar — escapes what the native encoding lacks
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_translateChar(x: Sexp) -> *const c_char {
    // the common case first: text already in the native encoding
    if x.typ() == CHARSXP && matches!(chars::to_native(x.gp(), x == R_NaString, locale()), Ok(Translation::None)) {
        return x.std_data() as *const c_char;
    }
    ret(translate_char(x))
}

// R: sysutils.c:translateCharFP — the conversion must work (file paths)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_translateCharFP(x: Sexp) -> *const c_char {
    ret(translate_char_as(x, "Rf_translateCharFP", 1))
}

/// translateChar0.
unsafe fn translate_char0(x: Sexp) -> Res<*const c_char> {
    check_charsxp(x, "Rf_translateChar0")?;
    if x.gp() & gp::BYTES != 0 {
        return Ok(x.std_data() as *const c_char);
    }
    translate_char(x)
}

// R: sysutils.c:translateChar0 — "bytes" strings as they are
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_translateChar0(x: Sexp) -> *const c_char {
    ret(translate_char0(x))
}

/// translateCharUTF8.
pub unsafe fn translate_char_utf8(x: Sexp) -> Res<*const c_char> {
    check_charsxp(x, "Rf_translateCharUTF8")?;
    let t = needs_translation_utf8(x)?;
    if t == Translation::None {
        return Ok(x.std_data() as *const c_char);
    }
    match translate_to_utf8(x.char_bytes(), t, 0)? {
        Some(v) => r_alloc_str(&v),
        None => Ok(std::ptr::null()),
    }
}

// R: sysutils.c:translateCharUTF8
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_translateCharUTF8(x: Sexp) -> *const c_char {
    ret(translate_char_utf8(x))
}

unsafe extern "C" {
    // robigo_glue.c (GNU R's util.c)
    fn Rf_AdobeSymbol2utf8(work: *mut c_char, c0: *const c_char, nwork: usize, use_pua: c_int) -> *mut c_void;
}

/// `x` converted from `fromcode` to `tocode`; what iconv cannot convert is
/// dropped (subst 0) or replaced byte by byte with `<xx>` (1), `.` (2) or
/// `?` (3). `None` when iconv has no such conversion.
/// R: sysutils.c:reEncodeIconv (not from wchar_t)
unsafe fn re_encode_iconv(x: &[u8], fromcode: &CStr, tocode: &CStr, subst: c_int) -> Option<Vec<u8>> {
    let cd = Riconv_open(tocode.as_ptr(), fromcode.as_ptr());
    if cd as isize == -1 {
        return None;
    }
    let out = convert(cd as libc::iconv_t, x, |rest, out| {
        match subst {
            1 => return subst_hex(rest, out),
            2 => out.push(b'.'),
            3 => out.push(b'?'),
            _ => {}
        }
        1
    });
    Riconv_close(cd);
    Some(out)
}

/// `None` when no conversion is needed or possible, as reEncode's 1.
/// R: sysutils.c:reEncode
unsafe fn re_encode(x: *const c_char, ce_in: c_int, ce_out: c_int, subst: c_int) -> Res<Option<*const c_char>> {
    // R: we can only encode from Symbol to UTF-8
    if ce_in == ce_out || ce_out == ce::SYMBOL || ce_in == ce::ANY || ce_out == ce::ANY {
        return Ok(None);
    }
    if ce_in == ce::SYMBOL {
        if ce_out != ce::UTF8 {
            return Ok(None);
        }
        let nc = 3 * CStr::from_ptr(x).to_bytes().len() + 1; // all in the BMP
        let p = r_alloc(nc, 1)?;
        Rf_AdobeSymbol2utf8(p, x, nc, 1);
        return Ok(Some(p));
    }
    let b = CStr::from_ptr(x).to_bytes();
    if b.is_ascii() {
        return Ok(None);
    }
    let l = locale();
    if l.utf8 && ((ce_in == ce::NATIVE && ce_out == ce::UTF8) || (ce_out == ce::NATIVE && ce_in == ce::UTF8)) {
        return Ok(None);
    }
    if l.latin1 && ((ce_in == ce::NATIVE && ce_out == ce::LATIN1) || (ce_out == ce::NATIVE && ce_in == ce::LATIN1)) {
        return Ok(None);
    }
    let fromcode = match ce_in {
        ce::NATIVE => c"",
        ce::LATIN1 => LATIN1_FROM,
        ce::UTF8 => c"UTF-8",
        _ => return Ok(None),
    };
    let tocode = match ce_out {
        ce::NATIVE => c"",
        ce::LATIN1 => c"latin1",
        ce::UTF8 => c"UTF-8",
        _ => return Ok(None),
    };
    match re_encode_iconv(b, fromcode, tocode, subst) {
        Some(v) => Ok(Some(r_alloc_str(&v)?)),
        None => Ok(None),
    }
}

/// reEnc.
pub unsafe fn re_enc(x: *const c_char, ce_in: c_int, ce_out: c_int, subst: c_int) -> Res<*const c_char> {
    Ok(re_encode(x, ce_in, ce_out, subst)?.unwrap_or(x))
}

// R: sysutils.c:reEnc
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_reEnc(x: *const c_char, ce_in: c_int, ce_out: c_int, subst: c_int) -> *const c_char {
    ret(re_enc(x, ce_in, ce_out, subst))
}

// R: sysutils.c:ucstoutf8
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ucstoutf8(s: *mut c_char, wc: c_uint) -> usize {
    ret(ucstoutf8(s, wc))
}

/// ucstoutf8.
unsafe fn ucstoutf8(s: *mut c_char, wc: c_uint) -> Res<usize> {
    if wc == 0 {
        *s = 0;
        return Ok(1);
    }
    let mut buf = [0u8; 16];
    let wcs = [wc, 0];
    let mut inbuf = wcs.as_ptr() as *mut c_char;
    let mut inbytesleft = std::mem::size_of::<c_uint>();
    let mut outbuf = buf.as_mut_ptr() as *mut c_char;
    let mut outbytesleft = buf.len();
    let cd = cached_iconv(UCS_UTF8, c"UTF-8", UNICODE)?;
    let status = libc::iconv(cd, &mut inbuf, &mut inbytesleft, &mut outbuf, &mut outbytesleft);
    if status == usize::MAX && errno() != libc::E2BIG {
        return Err(error_str(&format!("invalid Unicode point {wc}")));
    }
    *outbuf = 0;
    let n = CStr::from_ptr(buf.as_ptr() as *const c_char).to_bytes().len();
    std::ptr::copy_nonoverlapping(buf.as_ptr(), s as *mut u8, n + 1);
    Ok(n)
}

/// "invalid multibyte string, %s" when `msg_name` is given, else `code`.
unsafe fn nchar_error(msg_name: *const c_char, what: &str, code: c_int) -> Res<c_int> {
    if msg_name.is_null() {
        return Ok(code);
    }
    let m = CStr::from_ptr(msg_name).to_string_lossy().into_owned();
    Err(error_str(&what.replace("%s", &m)))
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
    ret(r_nchar(string, type_, allow_na, keep_na, msg_name))
}

/// R_nchar.
unsafe fn r_nchar(string: Sexp, type_: c_int, allow_na: c_int, keep_na: c_int, msg_name: *const c_char) -> Res<c_int> {
    const NA_INTEGER: c_int = i32::MIN;
    const INVALID: &str = "invalid multibyte string, %s";
    if string == R_NaString {
        return Ok(if keep_na != 0 { NA_INTEGER } else { 2 });
    }
    let g = string.gp();
    let p = string.std_data() as *const c_char;
    let utf8_chars = |width: bool| -> c_int {
        let mut nc = 0;
        let mut q = p;
        while *q != 0 {
            if width {
                let mut wc: libc::wchar_t = 0;
                Rf_utf8toucs(&mut wc, q);
                nc += Ri18n_wcwidth(wc);
            } else {
                nc += 1;
            }
            q = q.add(utf8clen(*q) as usize);
        }
        nc
    };
    // the native string as wide characters, or None when it is invalid
    let native_wide = || -> Res<Option<Vec<libc::wchar_t>>> {
        let xi = translate_char(string)?;
        let nc = mbstowcs(std::ptr::null_mut(), xi, 0);
        if nc == usize::MAX {
            return Ok(None);
        }
        let mut wc = vec![0 as libc::wchar_t; nc + 1];
        mbstowcs(wc.as_mut_ptr(), xi, nc + 1);
        wc.truncate(nc);
        Ok(Some(wc))
    };
    let mbcs = mbcslocale != 0;
    Ok(match type_ {
        0 => length(string)?,
        1 => {
            if g & gp::UTF8 != 0 {
                if utf8Valid(p) == 0 {
                    if allow_na == 0 {
                        return nchar_error(msg_name, INVALID, -1);
                    }
                    return Ok(NA_INTEGER);
                }
                utf8_chars(false)
            } else if g & gp::LATIN1 != 0 {
                CStr::from_ptr(p).to_bytes().len() as c_int
            } else if g & gp::BYTES != 0 {
                if allow_na == 0 {
                    return nchar_error(
                        msg_name,
                        "number of characters is not computable in \"bytes\" encoding, %s",
                        -2,
                    );
                }
                NA_INTEGER
            } else if mbcs {
                let nc = mbstowcs(std::ptr::null_mut(), translate_char(string)?, 0) as isize as c_int;
                if allow_na == 0 && nc < 0 {
                    return nchar_error(msg_name, INVALID, -1);
                }
                if nc >= 0 { nc } else { NA_INTEGER }
            } else {
                CStr::from_ptr(translate_char(string)?).to_bytes().len() as c_int
            }
        }
        2 => {
            if g & gp::UTF8 != 0 {
                if utf8Valid(p) == 0 {
                    if allow_na == 0 {
                        return nchar_error(msg_name, INVALID, -1);
                    }
                    return Ok(NA_INTEGER);
                }
                utf8_chars(true)
            } else if g & gp::BYTES != 0 {
                if allow_na == 0 {
                    return nchar_error(msg_name, "width is not computable for %s in \"bytes\" encoding", -2);
                }
                NA_INTEGER
            } else if g & gp::LATIN1 != 0 {
                CStr::from_ptr(p).to_bytes().len() as c_int
            } else if mbcs {
                match native_wide()? {
                    Some(wc) => {
                        let mut z = wc;
                        z.push(0);
                        let nci18n = Ri18n_wcswidth(z.as_ptr(), 2147483647);
                        if nci18n < 0 { (z.len() - 1) as c_int } else { nci18n }
                    }
                    None if allow_na == 0 => nchar_error(msg_name, INVALID, -1)?,
                    None => NA_INTEGER,
                }
            } else {
                CStr::from_ptr(translate_char(string)?).to_bytes().len() as c_int
            }
        }
        _ => NA_INTEGER,
    })
}

// ---- symbols (names.c) --------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_install(name: *const c_char) -> Sexp {
    let b = CStr::from_ptr(name).to_bytes();
    if b.is_empty() || b.len() > 10000 {
        throw(install_error(b.len()))
    }
    core().install(b)
}

#[cold]
unsafe fn install_error(n: usize) -> Unwind {
    error_str(if n == 0 {
        "attempt to use zero-length variable name"
    } else {
        "variable names are limited to 10000 bytes"
    })
}

/// install.
#[inline]
pub unsafe fn install(name: *const c_char) -> Res<Sexp> {
    let b = CStr::from_ptr(name).to_bytes();
    if b.is_empty() || b.len() > 10000 {
        return Err(install_error(b.len()));
    }
    Ok(core().install(b))
}

// R: sysutils.c:Rf_installChar — since R 4.0.0 the same as installTrChar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_installChar(x: Sexp) -> Sexp {
    ret(install_tr_char(x))
}

// R: sysutils.c:installTrChar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_installTrChar(x: Sexp) -> Sexp {
    ret(install_tr_char(x))
}

/// installTrChar.
pub unsafe fn install_tr_char(x: Sexp) -> Res<Sexp> {
    check_charsxp(x, "Rf_installTrChar")?;
    let t = needs_translation(x)?;
    if t == Translation::None {
        return install_no_tr_char(x);
    }
    // R: for back-compatibility this installs symbols with escapes, with a
    // warning
    let (b, failed) = translate_to_native(x.char_bytes(), t)?;
    if failed {
        must_work_native(&b, 2)?;
    }
    let c = std::ffi::CString::new(b).unwrap_or_default();
    install(c.as_ptr())
}

// R: names.c:installNoTrChar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_installNoTrChar(x: Sexp) -> Sexp {
    ret(install_no_tr_char(x))
}

/// installNoTrChar: the bytes, whatever their encoding. GNU R checks the
/// length only for a new symbol; no existing one is empty or longer than
/// MAXIDSIZE, so checking first is the same.
pub unsafe fn install_no_tr_char(x: Sexp) -> Res<Sexp> {
    let b = x.char_bytes();
    if b.is_empty() || b.len() > 10000 {
        return Err(install_error(b.len()));
    }
    // R: a new symbol's PRINTNAME is the CHARSXP itself when its text is
    // ASCII or in the locale's encoding, else mkChar(CHAR(x))
    let g = x.gp();
    let l = locale();
    Ok(if g & gp::ASCII != 0 || (g & gp::UTF8 != 0 && l.utf8) || (g & gp::LATIN1 != 0 && l.latin1) {
        core().install_char(x)
    } else {
        core().install(b)
    })
}

// ---- iconv (sysutils.c Riconv*) -------------------------------------------------

// R: sysutils.c:Riconv_open (not Windows or macOS): "" is the locale's
// encoding, and "utf8", which is not valid but people keep on using it, is
// "UTF-8"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Riconv_open(to: *const c_char, from: *const c_char) -> *mut c_void {
    let fix = |s: *const c_char| -> *const c_char {
        if CStr::from_ptr(s).to_bytes().eq_ignore_ascii_case(b"utf8") { c"UTF-8".as_ptr() } else { s }
    };
    libc::iconv_open(fix(to), fix(from))
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
