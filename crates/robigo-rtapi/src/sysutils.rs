//! sysutils.c's and util.c's exports that GNU R's base-package natives use
//! (the A5 note): string translation (`translateChar`,
//! `translateCharUTF8`, `getCharCE`, `installTrChar`), `Mbrtowc`, and the
//! file helpers `R_FileExists` and `RC_fopen`.

use crate::entry;
use crate::memory::r_alloc_cstr;
use robigo_rtcore::{Char, Cx, Encoding, RResult, Root, Sexp};
use std::borrow::Cow;
use std::ffi::{CStr, CString, c_char, c_int, c_void};

/// `CHECK_CHARSXP(x)` in function `f`: the CHARSXP `x` is.
// R: sysutils.c:CHECK_CHARSXP
fn check_charsxp(cx: &mut Cx, x: Root<'_>, f: &str) -> RResult<()> {
    let r = x.bind(cx);
    if r.as_char().is_some() {
        return Ok(());
    }
    let t = robigo_rtbuiltins::util::type_to_char(r);
    Err(cx.error(&format!("'{f}' must be called on a CHARSXP, but got '{t}'")))
}

/// The CHARSXP `x` (checked) translated by `f` in the isolate's locale.
fn translated<T>(cx: &Cx, x: Root<'_>, f: impl FnOnce(Char<'_>, robigo_rtcore::Locale) -> T) -> T {
    let loc = cx.locale();
    let c: Char<'_> = x.bind(cx).as_char().unwrap();
    f(c, loc)
}

/// `translateChar(c)`: None when the string is already native.
fn native(c: Char<'_>, loc: robigo_rtcore::Locale) -> Result<Option<Vec<u8>>, String> {
    robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), loc).map(owned).map_err(|e| e.to_string())
}

/// What a translation gives C: `CHAR(x)` when nothing changes, else a copy
/// on the R_alloc stack (`copyAndFreeStringBuffer`).
fn hand_out(
    cx: &mut Cx,
    s: robigo_rtcore::Scope<'_>,
    x: Root<'_>,
    t: Result<Option<Vec<u8>>, String>,
) -> RResult<*const c_char> {
    match t {
        Ok(None) => {
            let c = x.bind(cx).as_char().unwrap();
            Ok(c.bytes().as_ptr().cast())
        }
        Ok(Some(b)) => r_alloc_cstr(cx, s, &b),
        Err(m) => Err(cx.error(&m)),
    }
}

/// A translation as [`hand_out`] takes it: none when it borrows.
fn owned(t: Cow<'_, [u8]>) -> Option<Vec<u8>> {
    match t {
        Cow::Borrowed(_) => None,
        Cow::Owned(v) => Some(v),
    }
}

entry! {
    /// `translateChar(x)`: the string in the native encoding (what cannot
    /// be converted escaped).
    // R: sysutils.c:translateChar
    [1] fn Rf_translateChar(x: Sexp) -> *const c_char => |cx, s| {
        check_charsxp(cx, x, "translateChar")?;
        let t = translated(cx, x, native);
        hand_out(cx, s, x, t)
    }

    /// `translateCharUTF8(x)`: the string in UTF-8.
    // R: sysutils.c:translateCharUTF8
    [1] fn Rf_translateCharUTF8(x: Sexp) -> *const c_char => |cx, s| {
        check_charsxp(cx, x, "translateCharUTF8")?;
        let t = translated(cx, x, |c, loc| {
            robigo_rtbuiltins::text::to_utf8(robigo_rtbuiltins::coerce::text(c), loc).map(owned).map_err(|e| e.to_string())
        });
        hand_out(cx, s, x, t)
    }

    /// `getCharCE(x)`: the declared encoding, as `cetype_t` (CE_NATIVE 0,
    /// CE_UTF8 1, CE_LATIN1 2, CE_BYTES 3).
    // R: sysutils.c:getCharCE
    [1] fn Rf_getCharCE(x: Sexp) -> c_int => |cx, _s| {
        check_charsxp(cx, x, "getCharCE")?;
        Ok(match x.bind(cx).as_char().unwrap().enc() {
            Encoding::Utf8 => 1,
            Encoding::Latin1 => 2,
            Encoding::Bytes => 3,
            Encoding::Native => 0,
        })
    }

    /// `installTrChar(x)`: the symbol of the string translated (with
    /// escapes where it cannot be: GNU R's back-compatible warning).
    // R: sysutils.c:installTrChar
    [1] fn Rf_installTrChar(x: Sexp) -> Sexp => |cx, s| {
        check_charsxp(cx, x, "installTrChar")?;
        // installTrChar's translation, warning and name checks (Sol S3)
        let r = {
            let c = x.bind(cx).as_char().unwrap();
            robigo_rtbuiltins::text::install_tr_name(robigo_rtbuiltins::coerce::text(c), cx.locale())
                .map(|(n, w)| (n.into_owned(), w))
        };
        let (name, w) = r.map_err(|d| cx.raise(d))?;
        if let Some(w) = w {
            cx.warning(&w)?;
        }
        crate::memory::alloc(cx, s, |cx| Ok(cx.install_tr(x.bind(cx).as_char().unwrap(), &name)?.into()))
    }

    /// `Mbrtowc(wc, s, n, ps)`: one character of `s` (at most `n` bytes)
    /// as a wide character in `*wc` (when `wc` is not null): the bytes it
    /// took, 0 at the end. An invalid or incomplete character is GNU R's
    /// error, the string shown with `<xx>` for the bytes that are not
    /// characters. robigo models UTF-8 and C `LC_CTYPE`s (a byte is a
    /// character in C); `ps` keeps no state between calls in either.
    // R: util.c:Mbrtowc
    [4] fn Rf_mbrtowc(wc: *mut c_void, s: *const c_char, n: usize, ps: *mut c_void) -> usize => |cx, _s| {
        let _ = ps;
        // SAFETY: C's contract: a NUL-terminated string.
        let bytes = unsafe { CStr::from_ptr(s) }.to_bytes();
        if n == 0 || bytes.is_empty() {
            return Ok(0);
        }
        let utf8 = cx.locale().utf8;
        match mb_char(&bytes[..bytes.len().min(n)], utf8) {
            Some((ch, used)) => {
                if !wc.is_null() {
                    // SAFETY: C's contract: room for a wchar_t (4 bytes).
                    unsafe { *wc.cast::<u32>() = ch };
                }
                Ok(used)
            }
            None => {
                // a readable version: the characters as they are, `<xx>`
                // for each byte that is not one
                // (n is GNU R's size_t: it counts down past 0 by wrapping,
                // and with no bytes left a character is incomplete)
                let mut err = Vec::new();
                let mut p = 0;
                let mut left = n;
                while p < bytes.len() {
                    match mb_char(&bytes[p..bytes.len().min(p.saturating_add(left))], utf8) {
                        Some((_, used)) => {
                            err.extend_from_slice(&bytes[p..p + used]);
                            p += used;
                            left = left.wrapping_sub(used);
                        }
                        None => {
                            err.extend_from_slice(format!("<{:02x}>", bytes[p]).as_bytes());
                            p += 1;
                            left = left.wrapping_sub(1);
                        }
                    }
                }
                Err(cx.error(&format!("invalid multibyte string at '{}'", String::from_utf8_lossy(&err))))
            }
        }
    }

    /// `R_FileExists(path)`: `stat` of the path, `~` expanded, succeeds.
    // R: sysutils.c:R_FileExists (Unix)
    [1] fn R_FileExists(path: *const c_char) -> c_int => |cx, _s| {
        // SAFETY: C's contract: a NUL-terminated string.
        let p = unsafe { CStr::from_ptr(path) }.to_bytes().to_vec();
        let p = robigo_rt::connections::r_expand_file_name(cx, &p)?;
        Ok(std::fs::metadata(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&p)).is_ok() as c_int)
    }

    /// `RC_fopen(fn, mode, expand)`: `fopen` of the file name `fn` (a
    /// CHARSXP) translated (`translateCharFP`), `~` expanded if asked; NULL
    /// for NA.
    // R: sysutils.c:RC_fopen
    [3] fn RC_fopen(fname: Sexp, mode: *const c_char, expand: c_int) -> *mut c_void => |cx, _s| {
        check_charsxp(cx, fname, "translateCharFP")?;
        if fname.bind(cx).sexp() == cx.na_string().as_ref().sexp() {
            return Ok(std::ptr::null_mut());
        }
        let name: Result<Vec<u8>, String> = translated(cx, fname, |c, loc| {
            robigo_rtbuiltins::text::to_native_fp(robigo_rtbuiltins::coerce::text(c), loc).map(|b| b.into_owned())
        });
        let name = match name {
            Ok(b) => b,
            Err(m) => return Err(cx.error(&m)),
        };
        let name = if expand != 0 { robigo_rt::connections::r_expand_file_name(cx, &name)? } else { name };
        let Ok(cname) = CString::new(name) else { return Ok(std::ptr::null_mut()) };
        // SAFETY: a NUL-terminated path and C's mode string.
        Ok(unsafe { libc::fopen(cname.as_ptr(), mode) }.cast())
    }
}

/// One character at the start of `b` in the locale: its code point and
/// length, None if `b` does not start with a whole valid one (mbrtowc's
/// (size_t) -1 and -2).
fn mb_char(b: &[u8], utf8: bool) -> Option<(u32, usize)> {
    // mbrtowc of no bytes: (size_t) -2, incomplete
    b.first()?;
    if !utf8 {
        return Some((b[0] as u32, 1));
    }
    let n = match b[0] {
        0x00..=0x7f => return Some((b[0] as u32, 1)),
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let s = std::str::from_utf8(b.get(..n)?).ok()?;
    Some((s.chars().next()? as u32, n))
}

#[cfg(test)]
mod tests {
    use super::mb_char;

    #[test]
    fn mb_char_reads_one_character() {
        assert_eq!(mb_char(b"a", true), Some((0x61, 1)));
        assert_eq!(mb_char("é".as_bytes(), true), Some((0xe9, 2)));
        assert_eq!(mb_char("€x".as_bytes(), true), Some((0x20ac, 3)));
        assert_eq!(mb_char(&[0xe2, 0x82], true), None);
        assert_eq!(mb_char(&[0xff], true), None);
        assert_eq!(mb_char(&[0xe9], false), Some((0xe9, 1)));
        assert_eq!(mb_char(&[], true), None);
    }
}
