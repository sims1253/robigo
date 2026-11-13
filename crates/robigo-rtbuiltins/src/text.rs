//! String comparison and translation as GNU R does them: `Seql`,
//! `Scollate` (C collation only, so far), `translateChar` and
//! `translateCharUTF8`, over `robigo-text`'s strings.

use robigo_rtcore::Locale;
use robigo_text::{Enc, Str, TranslateError, translate_char};
use std::borrow::Cow;
use std::cmp::Ordering;

/// `robigo-text`'s view of the isolate's `LC_CTYPE`.
#[inline]
pub fn ctype(loc: Locale) -> robigo_text::Locale {
    if loc.utf8 { robigo_text::Locale::Utf8 } else { robigo_text::Locale::C }
}

/// GNU R's `ENC_KNOWN`: marked latin1 or UTF-8 (the mask, so two marks
/// compare).
fn enc_known(s: Str<'_>) -> u8 {
    match s.enc() {
        Enc::Latin1 => 1,
        Enc::Utf8 => 2,
        _ => 0,
    }
}

/// `translateCharUTF8(s)`: `s` in UTF-8 (what iconv cannot convert written
/// `<xx>`). A "bytes" string is GNU R's error.
// R: sysutils.c:translateCharUTF8, needsTranslationUTF8, translateToUTF8
pub fn to_utf8(s: Str<'_>, loc: Locale) -> Result<Cow<'_, [u8]>, TranslateError> {
    if s.is_ascii() {
        return Ok(Cow::Borrowed(s.bytes()));
    }
    match s.enc() {
        Enc::Utf8 => Ok(Cow::Borrowed(s.bytes())),
        Enc::Bytes => Err(TranslateError),
        // CP1252 to UTF-8, as translate_char does into a UTF-8 locale
        Enc::Latin1 => translate_char(s, robigo_text::Locale::Utf8),
        Enc::Native if loc.utf8 => Ok(Cow::Borrowed(s.bytes())),
        // iconv from the C locale's ASCII: every high byte fails
        Enc::Native => {
            let mut out = Vec::with_capacity(s.bytes().len() + 8);
            for &b in s.bytes() {
                if b < 0x80 {
                    out.push(b);
                } else {
                    out.extend_from_slice(format!("<{b:02x}>").as_bytes());
                }
            }
            Ok(Cow::Owned(out))
        }
    }
}

/// `translateChar(s)`: `s` in the native encoding.
// R: sysutils.c:translateChar
#[inline]
pub fn to_native(s: Str<'_>, loc: Locale) -> Result<Cow<'_, [u8]>, TranslateError> {
    translate_char(s, ctype(loc))
}

/// GNU R's `Seql(a, b)`: the same CHARSXP, or, for strings whose
/// `ENC_KNOWN` differ and neither "bytes", the same text in UTF-8. (Two
/// different cached CHARSXPs with the same `ENC_KNOWN` differ.)
// R: memory.c:Seql
pub fn seql(a: Str<'_>, b: Str<'_>, loc: Locale) -> bool {
    // a CHARSXP is its bytes and its mark (ASCII never carries one)
    if a.bytes() == b.bytes() && a.enc() == b.enc() {
        return true;
    }
    // two cached CHARSXPs whose ENC_KNOWN agree ("bytes" counts as none)
    if enc_known(a) == enc_known(b) {
        return false;
    }
    if a.enc() == Enc::Bytes || b.enc() == Enc::Bytes {
        return false;
    }
    match (to_utf8(a, loc), to_utf8(b, loc)) {
        (Ok(p), Ok(q)) => p == q,
        _ => false,
    }
}

/// Why `Scollate` failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollateError {
    /// `translateChar` of a "bytes" string.
    Bytes,
    /// The collation is ICU's (not implemented yet).
    NotC,
}

/// GNU R's `Scollate(a, b)` under C collation: `strcmp` of the native
/// translations.
// R: util.c:Scollate
pub fn scollate(a: Str<'_>, b: Str<'_>, loc: Locale) -> Result<Ordering, CollateError> {
    if !loc.collate_c {
        return Err(CollateError::NotC);
    }
    let p = to_native(a, loc).map_err(|_| CollateError::Bytes)?;
    let q = to_native(b, loc).map_err(|_| CollateError::Bytes)?;
    Ok(p.as_ref().cmp(q.as_ref()))
}
