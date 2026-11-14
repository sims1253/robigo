//! A CHARSXP's bytes and encoding mark, and what the C library and R's
//! `rlocale.c` say about characters in the locale: `mbrtowc`, `iswprint`,
//! `isprint`, `Ri18n_wcwidth`, and `translateChar` (sysutils.c).

use std::borrow::Cow;

use crate::tables::{ISWPRINT_C_UTF8, WCWIDTH, ZERO_WIDTH};

/// The C library's `LC_CTYPE`, as far as the ports read it.
///
/// Other locales (Latin-1 or CJK multibyte ones) are not modelled: the
/// caller reports them as not implemented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    /// A UTF-8 locale outside East Asia (`C.UTF-8`, `en_US.UTF-8`):
    /// `mbcslocale` and `utf8locale`, glibc's `iswprint`, and R's
    /// `Ri18n_wcwidth` with its default column.
    Utf8,
    /// `C`/`POSIX`: single-byte ASCII; nothing above 0x7f is printable.
    C,
}

impl Locale {
    /// R's `mbcslocale`.
    #[inline]
    pub fn mbcs(self) -> bool {
        self == Locale::Utf8
    }

    /// R's `utf8locale`.
    #[inline]
    pub fn utf8(self) -> bool {
        self == Locale::Utf8
    }
}

/// A CHARSXP's encoding mark (`getCharCE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enc {
    /// `CE_NATIVE`: no mark; the bytes are in the locale's encoding.
    Native,
    /// `CE_UTF8`.
    Utf8,
    /// `CE_LATIN1` (translated as CP1252, as R does where iconv has it).
    Latin1,
    /// `CE_BYTES`.
    Bytes,
}

/// A string as a CHARSXP holds it: bytes (without NULs) and the mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Str<'a> {
    bytes: &'a [u8],
    enc: Enc,
}

impl<'a> Str<'a> {
    /// `mkCharLenCE(bytes, enc)`: an ASCII string carries no mark.
    // R: envir.c:mkCharLenCE
    #[inline]
    pub fn new(bytes: &'a [u8], enc: Enc) -> Str<'a> {
        let enc = if enc != Enc::Native && bytes.is_ascii() { Enc::Native } else { enc };
        Str { bytes, enc }
    }

    /// An ASCII string (in a `const`).
    pub const fn ascii(bytes: &'a [u8]) -> Str<'a> {
        assert!(bytes.is_ascii(), "Str::ascii of non-ASCII bytes");
        Str { bytes, enc: Enc::Native }
    }

    /// `CHAR(s)`.
    #[inline]
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// `getCharCE(s)`.
    #[inline]
    pub fn enc(&self) -> Enc {
        self.enc
    }

    /// `IS_ASCII(s)`.
    #[inline]
    pub fn is_ascii(&self) -> bool {
        self.bytes.is_ascii()
    }
}

/// The byte at `i`, or the C string's terminator past the end. The ports
/// read past the end where GNU R does (see `EncodeString`); a CHARSXP's
/// bytes are followed by its NUL, and anything further is taken as NUL
/// too.
#[inline]
pub(crate) fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `utf8toucs(&wc, s + i)`: R's own decoder of one UTF-8 character, which
/// lets through overlong forms and values above U+10FFFF. Returns the
/// length (0 for the terminator; -1 invalid, -2 incomplete) and `wc`.
// R: util.c:utf8toucs
pub fn utf8toucs(s: &[u8], i: usize) -> (i32, i32) {
    let byte = at(s, i) as u32;
    let c = |k: usize| at(s, i + k) as u32;
    // strlen(s): the bytes before the terminator.
    let len = s.len().saturating_sub(i);
    let cont = |k: usize| c(k) & 0xC0 == 0x80;
    if byte == 0 {
        (0, 0)
    } else if byte < 0x80 {
        (1, byte as i32)
    } else if byte < 0xC0 {
        (-1, 0)
    } else if byte < 0xE0 {
        if len < 2 {
            return (-2, 0);
        }
        if cont(1) { (2, (((byte & 0x1F) << 6) | (c(1) & 0x3F)) as i32) } else { (-1, 0) }
    } else if byte < 0xF0 {
        if len < 3 {
            return (-2, 0);
        }
        if cont(1) && cont(2) {
            let w = ((byte & 0x0F) << 12) | ((c(1) & 0x3F) << 6) | (c(2) & 0x3F);
            // Surrogates range
            if (0xD800..=0xDFFF).contains(&w) || w == 0xFFFE || w == 0xFFFF {
                return (-1, 0);
            }
            (3, w as i32)
        } else {
            (-1, 0)
        }
    } else if byte < 0xF8 {
        if len < 4 {
            return (-2, 0);
        }
        if cont(1) && cont(2) && cont(3) {
            let w = ((byte & 0x0F) << 18) | ((c(1) & 0x3F) << 12) | ((c(2) & 0x3F) << 6) | (c(3) & 0x3F);
            (4, w as i32)
        } else {
            (-1, 0)
        }
    } else if byte < 0xFC {
        // So now handle 5.6 byte sequences with no testing
        if len < 5 {
            return (-2, 0);
        }
        let w = ((byte & 0x0F) << 24)
            | ((c(1) & 0x3F) << 12)
            | ((c(2) & 0x3F) << 12)
            | ((c(3) & 0x3F) << 6)
            | (c(4) & 0x3F);
        (5, w as i32)
    } else {
        if len < 6 {
            return (-2, 0);
        }
        // (int shifts that overflow wrap, as compiled)
        let w = ((byte & 0x0F).wrapping_shl(30))
            | ((c(1) & 0x3F) << 24)
            | ((c(2) & 0x3F) << 18)
            | ((c(3) & 0x3F) << 12)
            | ((c(4) & 0x3F) << 6)
            | (c(5) & 0x3F);
        (6, w as i32)
    }
}

/// `IS_HIGH_SURROGATE(wc)`.
#[inline]
pub fn is_high_surrogate(wc: i32) -> bool {
    (0xD800..=0xDBFF).contains(&wc)
}

/// `utf8toucs32(high, s + i)`: the code point of a surrogate pair whose
/// low half is read from bytes 2 and 3 (only reached through `utf8toucs`'s
/// unchecked six-byte forms with a 32-bit `wchar_t`).
// R: util.c:utf8toucs32
pub fn utf8toucs32(high: i32, s: &[u8], i: usize) -> u32 {
    let low = 0xDC00 | ((at(s, i + 2) as u32 & 0x0F) << 6) | (at(s, i + 3) as u32 & 0x3F);
    0x10000u32.wrapping_add(((high as u32) & 0x3FF) << 10).wrapping_add(low & 0x3FF)
}

/// `mbrtowc(&wc, s + i, R_MB_CUR_MAX, &mb_st)` in a UTF-8 locale: glibc's
/// decoder, which rejects overlong forms and surrogates but accepts the
/// old five- and six-byte forms and values above U+10FFFF (EncodeString
/// then rejects those; Rstrwid counts them as `\U` escapes). A sequence
/// cut short meets the terminator and is invalid. Returns the length (0
/// for the terminator, -1 invalid) and `wc`.
pub fn mbrtowc_utf8(s: &[u8], i: usize) -> (i32, i32) {
    let b0 = at(s, i) as u32;
    let (n, min, init) = match b0 {
        0 => return (0, 0),
        0x01..=0x7F => return (1, b0 as i32),
        0xC2..=0xDF => (2, 0x80, b0 & 0x1F),
        0xE0..=0xEF => (3, 0x800, b0 & 0x0F),
        0xF0..=0xF7 => (4, 0x10000, b0 & 0x07),
        0xF8..=0xFB => (5, 0x20_0000, b0 & 0x03),
        0xFC..=0xFD => (6, 0x400_0000, b0 & 0x01),
        _ => return (-1, 0),
    };
    let mut wc = init;
    for k in 1..n {
        let c = at(s, i + k) as u32;
        if c & 0xC0 != 0x80 {
            return (-1, 0);
        }
        wc = (wc << 6) | (c & 0x3F);
    }
    if wc < min || (0xD800..=0xDFFF).contains(&wc) {
        return (-1, 0);
    }
    (n as i32, wc as i32)
}

/// The locale's `mbrtowc` (only called in a multibyte locale).
#[inline]
pub(crate) fn mbrtowc(s: &[u8], i: usize, loc: Locale) -> (i32, i32) {
    debug_assert!(loc.mbcs());
    mbrtowc_utf8(s, i)
}

/// `iswprint(k)`.
pub(crate) fn iswprint(k: u32, loc: Locale) -> bool {
    match loc {
        Locale::C => (0x20..0x7F).contains(&k),
        Locale::Utf8 => in_ranges(k, &ISWPRINT_C_UTF8),
    }
}

/// `isprint(c)` of a byte (single-byte locales).
#[inline]
pub(crate) fn isprint(c: u8, loc: Locale) -> bool {
    match loc {
        // In a UTF-8 locale no byte above 0x7f is a character.
        Locale::C | Locale::Utf8 => (0x20..0x7F).contains(&c),
    }
}

fn in_ranges(k: u32, table: &[(u32, u32)]) -> bool {
    table
        .binary_search_by(|&(a, b)| {
            if b < k {
                std::cmp::Ordering::Less
            } else if a > k {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `Ri18n_wcwidth(c)`: R's own width table (R is configured with
/// `USE_RI18N_WIDTH` on Linux), column `MB_Default`.
// R: rlocale.c:Ri18n_wcwidth, wcwidthsearch
pub fn ri18n_wcwidth(c: u32) -> i32 {
    // This quickly gives one for printing ASCII characters
    if c > 0x1F && c < 0x7F {
        return 1;
    }
    let wd = match WCWIDTH.binary_search_by(|&(a, b, _)| {
        if b < c {
            std::cmp::Ordering::Less
        } else if a > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(i) => WCWIDTH[i].2 as i32,
        Err(_) => -1,
    };
    if wd >= 0 {
        return wd; // currently all are 1 or 2.
    }
    // assume unknown chars are width one.
    if in_ranges(c, &ZERO_WIDTH) { 0 } else { 1 }
}

/// `translateChar` of a string marked "bytes".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranslateError;

impl std::fmt::Display for TranslateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("translating strings with \"bytes\" encoding is not allowed")
    }
}

impl std::error::Error for TranslateError {}

/// CP1252's 0x80..0x9f (glibc's iconv; `None` is unassigned).
const CP1252_HIGH: [Option<u16>; 32] = [
    Some(0x20AC),
    None,
    Some(0x201A),
    Some(0x0192),
    Some(0x201E),
    Some(0x2026),
    Some(0x2020),
    Some(0x2021),
    Some(0x02C6),
    Some(0x2030),
    Some(0x0160),
    Some(0x2039),
    Some(0x0152),
    None,
    Some(0x017D),
    None,
    None,
    Some(0x2018),
    Some(0x2019),
    Some(0x201C),
    Some(0x201D),
    Some(0x2022),
    Some(0x2013),
    Some(0x2014),
    Some(0x02DC),
    Some(0x2122),
    Some(0x0161),
    Some(0x203A),
    Some(0x0153),
    None,
    Some(0x017E),
    Some(0x0178),
];

fn valid_utf8(b: &[u8]) -> bool {
    std::str::from_utf8(b).is_ok()
}

/// `translateChar(s)`: `s` in the native encoding, with what iconv cannot
/// convert written as `<U+XXXX>` (from UTF-8) or `<xx>`. Borrowed when no
/// translation is needed (`p == CHAR(s)` in the C).
// R: sysutils.c:translateChar, needsTranslation, translateToNative
pub fn translate_char<'a>(s: Str<'a>, loc: Locale) -> Result<Cow<'a, [u8]>, TranslateError> {
    let p = s.bytes();
    // (NA_STRING is never marked.)
    let from_utf8 = match s.enc() {
        // (a native string needs no scan: it is returned as it is)
        Enc::Native => return Ok(Cow::Borrowed(p)),
        _ if s.is_ascii() => return Ok(Cow::Borrowed(p)),
        Enc::Bytes => return Err(TranslateError),
        Enc::Utf8 if loc.utf8() => return Ok(Cow::Borrowed(p)),
        Enc::Utf8 => true,
        Enc::Latin1 => false,
    };
    let mut out = Vec::with_capacity(p.len() + 8);
    let mut i = 0;
    while i < p.len() {
        let b = p[i];
        if b < 0x80 {
            out.push(b);
            i += 1;
            continue;
        }
        if !from_utf8 && loc.utf8() {
            // CP1252 to UTF-8
            let u = if b < 0xA0 { CP1252_HIGH[(b - 0x80) as usize] } else { Some(b as u16) };
            if let Some(u) = u {
                let mut buf = [0u8; 4];
                out.extend_from_slice(char::from_u32(u as u32).expect("a BMP scalar").encode_utf8(&mut buf).as_bytes());
                i += 1;
                continue;
            }
        }
        // iconv stops with EILSEQ: write the escape and go on
        if from_utf8 {
            // if starting in UTF-8, use \uxxxx
            let (clen, wc) = utf8toucs(p, i);
            if clen > 0 && p.len() - i >= clen as usize {
                let ucs = if is_high_surrogate(wc) { utf8toucs32(wc, p, i) } else { wc as u32 };
                i += clen as usize;
                if (0xE0000..=0xE007F).contains(&ucs) && valid_utf8(&p[i - clen as usize..i]) {
                    // glibc's iconv drops the Unicode tag characters when
                    // the target has none (iconv/loop.c,
                    // STANDARD_TO_LOOP_ERR_HANDLER), without an error
                    continue;
                }
                if ucs < 65536 {
                    out.extend_from_slice(format!("<U+{:04X}>", ucs as u16).as_bytes());
                } else {
                    out.extend_from_slice(format!("<U+{ucs:08X}>").as_bytes());
                }
                continue;
            }
        }
        out.extend_from_slice(format!("<{b:02x}>").as_bytes());
        i += 1;
    }
    Ok(Cow::Owned(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoders() {
        assert_eq!(utf8toucs(b"\xc3\xa9", 0), (2, 0xe9));
        assert_eq!(utf8toucs(b"\xc1\x81", 0), (2, 0x41)); // overlong
        assert_eq!(utf8toucs(b"\xed\xa0\x80", 0), (-1, 0));
        assert_eq!(utf8toucs(b"\xef\xbf\xbe", 0), (-1, 0));
        assert_eq!(utf8toucs(b"\xf4\x90\x80\x80", 0), (4, 0x110000));
        assert_eq!(utf8toucs(b"\xe2\x82", 0), (-2, 0));
        assert_eq!(mbrtowc_utf8(b"\xc1\x81", 0), (-1, 0));
        assert_eq!(mbrtowc_utf8(b"\xef\xbf\xbe", 0), (3, 0xfffe));
        assert_eq!(mbrtowc_utf8(b"\xf0\x9f\x98\x80", 0), (4, 0x1f600));
        assert_eq!(mbrtowc_utf8(b"\xe2\x82", 0), (-1, 0));
        assert_eq!(mbrtowc_utf8(b"\xf4\x90\x80\x80", 0), (4, 0x110000));
        assert_eq!(mbrtowc_utf8(b"\xf8\x88\x80\x80\x80", 0), (5, 0x200000));
        assert_eq!(mbrtowc_utf8(b"\xed\xa0\x80", 0), (-1, 0));
        assert_eq!(mbrtowc_utf8(b"", 0), (0, 0));
    }

    #[test]
    fn widths() {
        assert_eq!(ri18n_wcwidth(0x41), 1);
        assert_eq!(ri18n_wcwidth(0x4e2d), 2);
        assert_eq!(ri18n_wcwidth(0x301), 0);
        assert_eq!(ri18n_wcwidth(0x1f600), 2);
        assert_eq!(ri18n_wcwidth(0xe9), 1);
    }

    #[test]
    fn translate() {
        let t = |b: &[u8], e, l| translate_char(Str::new(b, e), l).unwrap().into_owned();
        assert_eq!(t(b"caf\xc3\xa9", Enc::Utf8, Locale::C), b"caf<U+00E9>");
        assert_eq!(t(b"\xf0\x9f\x98\x80", Enc::Utf8, Locale::C), b"<U+0001F600>");
        assert_eq!(t(b"\xe9\x80\x81", Enc::Latin1, Locale::Utf8), "\u{e9}\u{20ac}<81>".as_bytes());
        assert_eq!(t(b"\xe9", Enc::Latin1, Locale::C), b"<e9>");
        assert_eq!(t(b"a\xf3\xa0\x80\x81b", Enc::Utf8, Locale::C), b"ab");
        assert_eq!(translate_char(Str::new(b"\xff", Enc::Bytes), Locale::Utf8), Err(TranslateError));
        assert!(matches!(translate_char(Str::new(b"\xc3\xa9", Enc::Utf8), Locale::Utf8), Ok(Cow::Borrowed(_))));
    }
}
