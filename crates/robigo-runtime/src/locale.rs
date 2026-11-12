//! The locale's character set as GNU R's string code sees it.
//!
//! GNU R keeps `utf8locale`, `latin1locale` and `mbcslocale` (and the
//! parser's `known_to_be_utf8`/`known_to_be_latin1`) as globals that
//! platform.c:R_check_locale sets at startup and after every
//! `Sys.setlocale`. Printing, `cat` and the parser consult them; in a
//! locale that is not UTF-8, strings marked UTF-8 or latin1 are translated
//! to the native encoding first (sysutils.c:translateChar), with
//! `<U+XXXX>` and `<xx>` for what that encoding cannot represent.
//!
//! robigo's text is Rust UTF-8 throughout, which the UTF-8 locales need no
//! work for: every check here is one relaxed load on that path, and the
//! translation itself is out of line. Native-encoding bytes that are not
//! ASCII travel through robigo's `String` output as the private-use
//! characters U+F780..U+F7FF ([`native_to_display`]), which
//! [`output_bytes`] turns back into those bytes when they reach a stream
//! or a connection.

use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering::Relaxed};

use crate::error::{Flow, RResult};
use crate::sys::Iconv;
use crate::value::{Encoding, RString};

const KNOWN: u8 = 1;
const UTF8: u8 = 2;
const LATIN1: u8 = 4;
const MBCS: u8 = 8;

/// The flags of the last [`check_locale`]; 0 until the first one.
static FLAGS: AtomicU8 = AtomicU8::new(0);
/// Counts [`check_locale`] calls, so cached conversions follow the locale
/// (R: sysutils.c:invalidate_cached_recodings).
static GENERATION: AtomicU32 = AtomicU32::new(0);

#[inline]
fn flags() -> u8 {
    let f = FLAGS.load(Relaxed);
    if f & KNOWN != 0 { f } else { flags_slow() }
}

#[cold]
#[inline(never)]
fn flags_slow() -> u8 {
    crate::builtins::sysutils::init_locale_on_evaluator();
    check_locale();
    FLAGS.load(Relaxed)
}

/// Rereads the C library's `LC_CTYPE`.
// R: platform.c:R_check_locale
pub fn check_locale() {
    let codeset = crate::sys::codeset();
    let mb_cur_max = crate::sys::mb_cur_max();
    let mut f = KNOWN;
    // more relaxed due to Darwin: CODESET is case-insensitive and latin1
    // is ISO8859-1
    if codeset.eq_ignore_ascii_case("UTF-8") {
        f |= UTF8;
    }
    if codeset == "ISO-8859-1" || codeset.eq_ignore_ascii_case("ISO8859-1") {
        f |= LATIN1;
    }
    // On Darwin 'regular' locales such as 'en_US' are UTF-8 but CODESET
    // is ""
    if cfg!(target_os = "macos") && codeset.is_empty() && (mb_cur_max == 4 || mb_cur_max == 6) {
        f |= UTF8;
    }
    if mb_cur_max > 1 {
        f |= MBCS;
    }
    FLAGS.store(f, Relaxed);
    GENERATION.fetch_add(1, Relaxed);
}

/// GNU R's `utf8locale` (and `known_to_be_utf8`).
#[inline]
pub fn utf8locale() -> bool {
    flags() & UTF8 != 0
}

/// GNU R's `latin1locale` (and `known_to_be_latin1`).
#[inline]
pub fn latin1locale() -> bool {
    flags() & LATIN1 != 0
}

/// GNU R's `mbcslocale`: `MB_CUR_MAX > 1`.
#[inline]
pub fn mbcslocale() -> bool {
    flags() & MBCS != 0
}

/// The iconv name of the native encoding.
fn native_codeset() -> String {
    if utf8locale() { "UTF-8".into() } else { crate::sys::codeset() }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum From {
    Utf8,
    Latin1,
}

// R: sysutils.c:needsTranslation
fn needs_translation(s: &RString) -> RResult<Option<From>> {
    Ok(match s.encoding() {
        Encoding::Native | Encoding::NativeInvalid => None,
        _ if s.is_ascii() => None,
        Encoding::Utf8 | Encoding::Utf8Invalid => (!utf8locale()).then_some(From::Utf8),
        Encoding::Latin1 => (!latin1locale()).then_some(From::Latin1),
        Encoding::Bytes => return Err(Flow::error("translating strings with \"bytes\" encoding is not allowed")),
    })
}

/// Whether [`translate_char`] would change `s`'s bytes.
pub fn translates(s: &RString) -> bool {
    !s.is_ascii()
        && match s.encoding() {
            Encoding::Utf8 | Encoding::Utf8Invalid => !utf8locale(),
            Encoding::Latin1 => !latin1locale(),
            _ => false,
        }
}

/// The bytes of `s` in the native encoding: UTF-8 and latin1 strings are
/// converted, with `<U+XXXX>` (`<U+XXXXXXXX>` past the BMP) for the
/// characters the locale lacks and `<xx>` for other bytes that fail.
// R: sysutils.c:translateChar
pub fn translate_char(s: &RString) -> RResult<Cow<'_, [u8]>> {
    match needs_translation(s)? {
        None => Ok(s.raw_bytes()),
        Some(from) => Ok(Cow::Owned(translate_to_native(&s.raw_bytes(), from)?)),
    }
}

thread_local! {
    /// The descriptors from UTF-8 and from CP1252 to the native encoding,
    /// for the locale of generation `.0` (R: sysutils.c's `utf8_obj` and
    /// `latin1_obj`).
    static TO_NATIVE: RefCell<(u32, Option<Iconv>, Option<Iconv>)> = const { RefCell::new((0, None, None)) };
}

// R: sysutils.c:translateToNative
#[cold]
#[inline(never)]
fn translate_to_native(ans: &[u8], ttype: From) -> RResult<Vec<u8>> {
    TO_NATIVE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let generation = GENERATION.load(Relaxed);
        if cache.0 != generation {
            *cache = (generation, None, None);
        }
        let (slot, from) = match ttype {
            // HAVE_ICONV_CP1252
            From::Latin1 => (&mut cache.2, "CP1252"),
            From::Utf8 => (&mut cache.1, "UTF-8"),
        };
        if slot.is_none() {
            let to = native_codeset();
            match Iconv::open(&to, from) {
                Some(cd) => *slot = Some(cd),
                // should never happen
                None => return Err(Flow::error(format!("unsupported conversion from '{from}' to ''"))),
            }
        }
        Ok(convert_substituting(slot.as_ref().unwrap(), ans, ttype == From::Utf8))
    })
}

/// CP1252's 0x80..0x9F range. Debian's GNU R is built with
/// `HAVE_ICONV_CP1252`, so `translateCharUTF8` translates latin1-marked
/// strings through CP1252, not ISO-8859-1 (sysutils.c:translateToUTF8).
/// The five undefined bytes are escaped as `<xx>` (mustWork = 0).
// R: sysutils.c:translateToUTF8
const CP1252_HIGH: [u32; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0, 0,
    0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

pub fn latin1_to_utf8(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        let cp = match b {
            0x80..=0x9F => CP1252_HIGH[(b - 0x80) as usize],
            0xA0..=0xFF => b as u32,
            _ => {
                out.push(b as char);
                continue;
            }
        };
        match char::from_u32(cp) {
            Some(_) if cp == 0 => out.push_str(&format!("<{b:02x}>")),
            Some(c) => out.push(c),
            None => out.push_str(&format!("<{b:02x}>")),
        }
    }
    out
}

/// The bytes of `s` in UTF-8, with `<xx>` for the bytes that fail.
// R: sysutils.c:translateCharUTF8, needsTranslationUTF8
pub fn translate_char_utf8(s: &RString) -> RResult<Cow<'_, [u8]>> {
    if s.is_ascii() {
        return Ok(Cow::Borrowed(s.as_bytes()));
    }
    match s.encoding() {
        Encoding::Utf8 | Encoding::Utf8Invalid => Ok(s.raw_bytes()),
        Encoding::Bytes => Err(Flow::error("translating strings with \"bytes\" encoding is not allowed")),
        Encoding::Latin1 => Ok(Cow::Owned(latin1_to_utf8(&s.raw_bytes()).into_bytes())),
        _ if latin1locale() => Ok(Cow::Owned(latin1_to_utf8(&s.raw_bytes()).into_bytes())),
        _ if utf8locale() => Ok(s.raw_bytes()),
        _ => Ok(Cow::Owned(native_to_utf8(&s.raw_bytes())?)),
    }
}

thread_local! {
    /// The descriptor from the native encoding to UTF-8, for the locale of
    /// generation `.0` (R's translateToUTF8 opens one per call).
    static FROM_NATIVE: RefCell<(u32, Option<Iconv>)> = const { RefCell::new((0, None)) };
}

/// Native-encoding bytes in UTF-8, with `<xx>` for the bytes that fail:
/// translateCharUTF8 of a native string in a locale that is neither UTF-8
/// nor latin1.
// R: sysutils.c:translateToUTF8 (NT_FROM_NATIVE, mustWork = 0)
#[cold]
#[inline(never)]
pub fn native_to_utf8(ans: &[u8]) -> RResult<Vec<u8>> {
    FROM_NATIVE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let generation = GENERATION.load(Relaxed);
        if cache.0 != generation || cache.1.is_none() {
            match Iconv::open("UTF-8", &native_codeset()) {
                Some(cd) => *cache = (generation, Some(cd)),
                None => return Err(Flow::error("unsupported conversion from '' to 'UTF-8'")),
            }
        }
        Ok(convert_substituting(cache.1.as_ref().unwrap(), ans, false))
    })
}

/// translateToNative's loop: `ans` through `cd`, substituting what fails.
fn convert_substituting(cd: &Iconv, ans: &[u8], from_utf8: bool) -> Vec<u8> {
    let mut buf = vec![0u8; (ans.len() * 4).max(64)];
    'top: loop {
        let size = buf.len();
        let mut inbuf: &[u8] = ans;
        let mut outbuf: &mut [u8] = &mut buf[..];
        // First initialize output
        cd.reset(&mut outbuf);
        loop {
            // Then convert input
            match cd.step(&mut inbuf, &mut outbuf) {
                Ok(()) => break,
                Err(libc::E2BIG) => {
                    buf.resize(2 * size, 0);
                    continue 'top;
                }
                Err(libc::EILSEQ) | Err(libc::EINVAL) => {
                    cd.reset(&mut outbuf);
                    if outbuf.len() < 13 {
                        buf.resize(2 * size, 0);
                        continue 'top;
                    }
                    let sub = if from_utf8 && let Some((ucs, clen)) = utf8_char(inbuf) {
                        inbuf = &inbuf[clen..];
                        if ucs < 65536 { format!("<U+{ucs:04X}>") } else { format!("<U+{ucs:08X}>") }
                    } else {
                        let s = format!("<{:02x}>", inbuf[0]);
                        inbuf = &inbuf[1..];
                        s
                    };
                    outbuf[..sub.len()].copy_from_slice(sub.as_bytes());
                    outbuf = &mut std::mem::take(&mut outbuf)[sub.len()..];
                }
                Err(_) => break,
            }
        }
        let used = size - outbuf.len();
        buf.truncate(used);
        return buf;
    }
}

/// The first UTF-8 character of `s` and its length, when complete.
fn utf8_char(s: &[u8]) -> Option<(u32, usize)> {
    let len = match *s.first()? {
        0..0x80 => 1,
        0xc0..0xe0 => 2,
        0xe0..0xf0 => 3,
        0xf0..0xf8 => 4,
        _ => return None,
    };
    let c = std::str::from_utf8(s.get(..len)?).ok()?.chars().next()?;
    Some((c as u32, len))
}

/// Native-encoding bytes as text for robigo's `String` output. In a UTF-8
/// locale they are that text; elsewhere each byte from 0x80 becomes the
/// private-use character U+F700 + byte, which [`output_bytes`] writes as
/// the byte again.
pub fn native_to_display(bytes: &[u8]) -> String {
    if let Ok(ascii) = std::str::from_utf8(bytes)
        && ascii.is_ascii()
    {
        return ascii.to_owned();
    }
    if utf8locale() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    bytes.iter().map(|&b| if b < 0x80 { b as char } else { char::from_u32(0xF700 + b as u32).unwrap() }).collect()
}

/// The bytes to write for output `text`: in a UTF-8 locale its UTF-8, and
/// otherwise with [`native_to_display`]'s characters as their bytes.
#[inline]
pub fn output_bytes(text: &str) -> Cow<'_, [u8]> {
    if utf8locale() || text.is_ascii() { Cow::Borrowed(text.as_bytes()) } else { output_bytes_slow(text) }
}

#[cold]
#[inline(never)]
fn output_bytes_slow(text: &str) -> Cow<'_, [u8]> {
    if !text.chars().any(is_display_byte) {
        return Cow::Borrowed(text.as_bytes());
    }
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        if is_display_byte(c) {
            out.push((c as u32 - 0xF700) as u8);
        } else {
            out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
        }
    }
    Cow::Owned(out)
}

/// Output-pipeline `text` (from [`native_to_display`], `EncodeString`,
/// deparse or a translated message) as an R string. Its private-use
/// characters become the native bytes again, marked native, as R's
/// `mkChar` of the native buffer. The private-use characters exist only
/// between translation and output, so every place where such text turns
/// into an R value goes through here.
#[inline]
pub fn display_to_rstring(text: &str) -> RString {
    if utf8locale() || text.is_ascii() { RString::from(text) } else { display_to_rstring_slow(text) }
}

#[cold]
#[inline(never)]
fn display_to_rstring_slow(text: &str) -> RString {
    match output_bytes_slow(text) {
        Cow::Owned(bytes) => RString::from_bytes(&bytes, Encoding::Native),
        Cow::Borrowed(_) => RString::from(text),
    }
}

/// [`display_to_rstring`] as a length-one character vector.
pub fn display_to_value(text: &str) -> crate::value::Value {
    crate::value::Value::strs(vec![Some(display_to_rstring(text))])
}

/// [`display_to_rstring`] of each line, as a character vector.
pub fn display_lines_value<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> crate::value::Value {
    crate::value::Value::strs(lines.into_iter().map(|l| Some(display_to_rstring(l.as_ref()))).collect())
}

fn is_display_byte(c: char) -> bool {
    ('\u{F780}'..='\u{F7FF}').contains(&c)
}

/// `s` as text for output in the native encoding: `cat`'s view of a
/// string (R: builtin.c:trChar). Bytes-encoded strings show their
/// non-ASCII bytes as `\xHH`.
pub fn display_native(s: &RString) -> RResult<String> {
    if s.is_bytes() {
        return Ok(bytes_escaped(&s.raw_bytes()));
    }
    Ok(native_to_display(&translate_char(s)?))
}

/// A bytes-encoded string's text with `\xHH` for each byte outside
/// 0x20..0x7f (R: builtin.c:trChar and the start of
/// printutils.c:EncodeString).
pub fn bytes_escaped(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &k in bytes {
        if (0x20..0x80).contains(&k) {
            out.push(k as char);
        } else {
            out.push_str(&format!("\\x{k:02x}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_chars() {
        assert_eq!(utf8_char("é!".as_bytes()), Some((0xe9, 2)));
        assert_eq!(utf8_char("😀".as_bytes()), Some((0x1f600, 4)));
        assert_eq!(utf8_char(b"\xc3"), None);
        assert_eq!(utf8_char(b"\xff"), None);
    }

    #[test]
    fn substitutions_into_ascii() {
        let cd = Iconv::open("ASCII", "UTF-8").unwrap();
        assert_eq!(convert_substituting(&cd, "café😀\u{20ac}".as_bytes(), true), b"caf<U+00E9><U+0001F600><U+20AC>");
        assert_eq!(convert_substituting(&cd, b"a\xffb", true), b"a<ff>b");
        let cd = Iconv::open("ASCII", "CP1252").unwrap();
        assert_eq!(convert_substituting(&cd, b"caf\xe9", false), b"caf<e9>");
    }

    #[test]
    fn display_bytes_round_trip() {
        assert_eq!(output_bytes_slow("a\u{F7E9}é"), Cow::<[u8]>::Owned(b"a\xe9\xc3\xa9".to_vec()));
        assert_eq!(bytes_escaped(b"a\xffb\\"), "a\\xffb\\");
    }
}
