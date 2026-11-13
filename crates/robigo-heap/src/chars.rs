//! The CHARSXP cache: a flat open-addressing table of `Sexp`s with linear
//! probing (council item 10). The 32-bit hash lives in each CHARSXP's
//! header (the attribute slot, which CHARSXPs do not use), so a probe
//! compares hashes before bytes and the table contains one word per entry.
//! Entries are weak: the collector prunes dead CHARSXPs by rebuilding
//! the table from the survivors (no tombstones).
//!
//! Below the table: GNU R's encoding rules for CHARSXPs (see "encoding
//! marks"), public so that the C API and the runtime share one copy.

use crate::Sexp;

pub(crate) struct CharTable {
    slots: Vec<Sexp>,
    len: usize,
}

const MIN_SLOTS: usize = 1024;

impl CharTable {
    pub fn new() -> CharTable {
        CharTable { slots: vec![Sexp::NULL; MIN_SLOTS], len: 0 }
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The entry with hash `h` for which `eq` is true.
    #[inline]
    pub fn find(&self, h: u32, mut eq: impl FnMut(Sexp) -> bool) -> Option<Sexp> {
        let mask = self.slots.len() - 1;
        let mut i = h as usize & mask;
        loop {
            // SAFETY: `i` is masked by `len - 1`, and the length is a power
            // of two. This is the CHARSXP cache's probe: `mkChar` is hot, and
            // a bounds check here costs 0.05% of `csv`.
            let c = unsafe { *self.slots.get_unchecked(i) };
            if c.is_null() {
                return None;
            }
            // SAFETY: entries are live CHARSXPs: `insert`'s contract, and the
            // collector prunes dead ones before it frees them.
            if unsafe { c.char_hash() } == h && eq(c) {
                return Some(c);
            }
            i = (i + 1) & mask;
        }
    }

    /// Insert a CHARSXP not in the table (its header contains its hash).
    ///
    /// # Safety
    /// `c` must be a live CHARSXP of the heap this table belongs to. The
    /// table reads its header from now on, until the collector prunes it.
    #[inline]
    pub unsafe fn insert(&mut self, c: Sexp) {
        if (self.len + 1) * 4 > self.slots.len() * 3 {
            self.rebuild(self.slots.len() * 2, |_| true);
        }
        // SAFETY: `place` asks for what this function's contract gives.
        unsafe { Self::place(&mut self.slots, c) };
        self.len += 1;
    }

    /// # Safety
    /// `c` must be a live CHARSXP: its header contains the hash.
    unsafe fn place(slots: &mut [Sexp], c: Sexp) {
        let mask = slots.len() - 1;
        // SAFETY: `c` is a live CHARSXP, this function's contract.
        let mut i = unsafe { c.char_hash() } as usize & mask;
        while !slots[i].is_null() {
            i = (i + 1) & mask;
        }
        slots[i] = c;
    }

    fn rebuild(&mut self, cap: usize, keep: impl Fn(Sexp) -> bool) {
        let old = std::mem::replace(&mut self.slots, vec![Sexp::NULL; cap]);
        self.len = 0;
        for c in old {
            if !c.is_null() && keep(c) {
                // SAFETY: an entry, so a live CHARSXP (see `find`): `keep` runs
                // after marking and before the sweep frees anything.
                unsafe { Self::place(&mut self.slots, c) };
                self.len += 1;
            }
        }
    }

    /// Whether `f` is true for every entry.
    pub fn all(&self, f: impl Fn(Sexp) -> bool) -> bool {
        self.slots.iter().all(|c| c.is_null() || f(*c))
    }

    /// Drop the entries `live` rejects (after marking, before any memory
    /// is reused); the table shrinks to fit the survivors.
    pub fn prune(&mut self, live: impl Fn(Sexp) -> bool) {
        let survivors = self.slots.iter().filter(|c| !c.is_null() && live(**c)).count();
        // the smallest table within insert's 3/4 load factor: a sparser
        // one costs a word per slot (10M strings: 128 MiB more, measured
        // by bench/scale.sh)
        let cap = (survivors * 4 / 3 + 1).next_power_of_two().max(MIN_SLOTS);
        self.rebuild(cap, live);
    }
}

impl Default for CharTable {
    fn default() -> Self {
        Self::new()
    }
}

// ---- encoding marks ---------------------------------------------------------
//
// GNU R's rules for a CHARSXP's encoding (envir.c:mkCharLenCE,
// sysutils.c:getCharCE and needsTranslation*), on the gp bits alone. They
// need no iconv and no C globals, so the C API and the runtime (G3) share
// them; the locale enters as a [`Locale`] value.

use crate::{Core, ce, gp};

/// The encoding marks of a CHARSXP (ASCII included).
pub const ENC_MASK: u16 = gp::UTF8 | gp::LATIN1 | gp::BYTES | gp::ASCII;

/// Why `mkCharLenCE` refuses its input. The C API turns each into GNU R's
/// message (`EmbeddedNul` needs EncodeString, which lives above the core).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MkCharError {
    /// "unknown encoding: %d"
    UnknownEncoding(i32),
    /// "embedded nul in string: '%s'"
    EmbeddedNul,
    /// "unknown encoding mask: %d": non-ASCII CE_SYMBOL or CE_ANY text
    /// that is not in the cache as a native string
    UnknownMask(i32),
}

/// The marks `mkCharLenCE` gives a non-ASCII string in encoding `enc`.
/// R: envir.c:mkCharLenCE (`need_enc`)
#[inline]
pub fn ce_bits(enc: i32) -> u16 {
    match enc {
        ce::UTF8 => gp::UTF8,
        ce::LATIN1 => gp::LATIN1,
        ce::BYTES => gp::BYTES,
        _ => 0,
    }
}

/// The `cetype_t` of a CHARSXP with gp bits `g`: UTF-8, then Latin-1, then
/// bytes, and native (`CE_NATIVE`, `Encoding()` "unknown") for an
/// unmarked string, ASCII ones included.
/// R: sysutils.c:getCharCE
#[inline]
pub fn char_ce(g: u16) -> i32 {
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

/// What GNU R's platform.c:R_check_locale found out about the C library's
/// locale; the native encoding is the locale's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locale {
    pub utf8: bool,
    pub latin1: bool,
}

/// Which conversion a CHARSXP needs (sysutils.c's `nttype_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Translation {
    /// the bytes are already in the target encoding
    None,
    FromUtf8,
    FromLatin1,
    FromNative,
}

/// A "bytes" string cannot be translated: GNU R's error is
/// `translating strings with "bytes" encoding is not allowed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BytesEncoding;

impl BytesEncoding {
    pub const MESSAGE: &'static str = "translating strings with \"bytes\" encoding is not allowed";
}

/// What translating a CHARSXP with gp bits `g` to the native encoding
/// needs; `is_na` is `x == NA_STRING`.
/// R: sysutils.c:needsTranslation
#[inline]
pub fn to_native(g: u16, is_na: bool, l: Locale) -> Result<Translation, BytesEncoding> {
    if g & gp::ASCII != 0 {
        return Ok(Translation::None);
    }
    if g & gp::UTF8 != 0 {
        return Ok(if l.utf8 || is_na { Translation::None } else { Translation::FromUtf8 });
    }
    if g & gp::LATIN1 != 0 {
        return Ok(if is_na || l.latin1 { Translation::None } else { Translation::FromLatin1 });
    }
    if g & gp::BYTES != 0 {
        return Err(BytesEncoding);
    }
    Ok(Translation::None)
}

/// What translating a CHARSXP with gp bits `g` to UTF-8 needs. A native
/// string in a Latin-1 locale is Latin-1; in a UTF-8 locale it is taken as
/// UTF-8 unchecked.
/// R: sysutils.c:needsTranslationUTF8
#[inline]
pub fn to_utf8(g: u16, is_na: bool, l: Locale) -> Result<Translation, BytesEncoding> {
    if g & (gp::UTF8 | gp::ASCII) != 0 || is_na {
        return Ok(Translation::None);
    }
    if g & gp::BYTES != 0 {
        return Err(BytesEncoding);
    }
    if g & gp::LATIN1 != 0 || l.latin1 {
        return Ok(Translation::FromLatin1);
    }
    if l.utf8 {
        return Ok(Translation::None);
    }
    Ok(Translation::FromNative)
}

/// R: sysutils.c:charIsUTF8
#[inline]
pub fn is_utf8(g: u16, is_na: bool, l: Locale) -> bool {
    if g & (gp::ASCII | gp::UTF8) != 0 {
        return true;
    }
    !(g & (gp::LATIN1 | gp::BYTES) != 0 || !l.utf8 || is_na)
}

/// R: sysutils.c:charIsLatin1
#[inline]
pub fn is_latin1(g: u16, is_na: bool, l: Locale) -> bool {
    if g & (gp::ASCII | gp::LATIN1) != 0 {
        return true;
    }
    !(!l.latin1 || g & (gp::UTF8 | gp::BYTES) != 0 || is_na)
}

impl Core {
    /// `mkCharLenCE(bytes, len, enc)`: the cached CHARSXP for `(bytes,
    /// encoding)`. ASCII text is always native and marked ASCII, whatever
    /// `enc` says (so CE_BYTES ASCII text has no BYTES mark); otherwise
    /// the string carries `enc`'s mark, and equal bytes in different
    /// encodings are different objects.
    /// R: envir.c:mkCharLenCE
    pub fn mk_char_ce(&mut self, bytes: &[u8], enc: i32) -> Result<Sexp, MkCharError> {
        if !matches!(enc, ce::NATIVE | ce::UTF8 | ce::LATIN1 | ce::BYTES | ce::SYMBOL | ce::ANY) {
            return Err(MkCharError::UnknownEncoding(enc));
        }
        if bytes.contains(&0) {
            return Err(MkCharError::EmbeddedNul);
        }
        if (enc == ce::SYMBOL || enc == ce::ANY) && !bytes.is_ascii() {
            // R: the cache is searched with need_enc 0 (a native string), and
            // only creating one reaches `error("unknown encoding mask")`.
            return self.find_char(bytes, 0).ok_or(MkCharError::UnknownMask(enc));
        }
        // `mk_char` drops the mark of ASCII text
        Ok(self.mk_char(bytes, ce_bits(enc)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UTF8_LOCALE: Locale = Locale { utf8: true, latin1: false };
    const LATIN1_LOCALE: Locale = Locale { utf8: false, latin1: true };
    const C_LOCALE: Locale = Locale { utf8: false, latin1: false };

    fn marks(x: Sexp) -> u16 {
        // SAFETY: the tests' CHARSXPs are fresh, and nothing collects.
        unsafe { x.gp() & ENC_MASK }
    }

    #[test]
    fn cache_identity_is_bytes_and_encoding() {
        let mut c = Core::new();
        let e = "\u{e9}".as_bytes();
        let native = c.mk_char_ce(e, ce::NATIVE).unwrap();
        let utf8 = c.mk_char_ce(e, ce::UTF8).unwrap();
        let latin1 = c.mk_char_ce(e, ce::LATIN1).unwrap();
        let bytes = c.mk_char_ce(e, ce::BYTES).unwrap();
        let all = [native, utf8, latin1, bytes];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "one object for two encodings");
            }
        }
        assert_eq!(native, c.mk_char_ce(e, ce::NATIVE).unwrap());
        assert_eq!(utf8, c.mk_char_ce(e, ce::UTF8).unwrap());
        assert_eq!(latin1, c.mk_char_ce(e, ce::LATIN1).unwrap());
        assert_eq!(bytes, c.mk_char_ce(e, ce::BYTES).unwrap());
        assert_eq!(native, c.mk_char(e, 0));
        // SAFETY: as in `marks`.
        let ce_of = |x: Sexp| char_ce(unsafe { x.gp() });
        assert_eq!(all.map(ce_of), [ce::NATIVE, ce::UTF8, ce::LATIN1, ce::BYTES]);
        assert_eq!(all.map(marks), [0, gp::UTF8, gp::LATIN1, gp::BYTES]);
    }

    #[test]
    fn ascii_text_is_native_in_every_encoding() {
        let mut c = Core::new();
        let a = c.mk_char_ce(b"abc", ce::NATIVE).unwrap();
        for enc in [ce::UTF8, ce::LATIN1, ce::BYTES, ce::SYMBOL, ce::ANY] {
            assert_eq!(c.mk_char_ce(b"abc", enc).unwrap(), a);
        }
        assert_eq!(marks(a), gp::ASCII);
        // SAFETY: as in `marks`.
        assert_eq!(char_ce(unsafe { a.gp() }), ce::NATIVE);
        assert_eq!(marks(c.mk_char_ce(b"", ce::UTF8).unwrap()), gp::ASCII);
        assert_eq!(marks(c.mk_char_ce(b"\x7f", ce::BYTES).unwrap()), gp::ASCII);
        assert_eq!(marks(c.mk_char_ce(b"\x80", ce::BYTES).unwrap()), gp::BYTES);
    }

    #[test]
    fn mk_char_ce_refuses_what_gnu_r_refuses() {
        let mut c = Core::new();
        assert_eq!(c.mk_char_ce(b"a", 4), Err(MkCharError::UnknownEncoding(4)));
        assert_eq!(c.mk_char_ce(b"a\0b", 7), Err(MkCharError::UnknownEncoding(7)));
        assert_eq!(c.mk_char_ce(b"a\0b", ce::UTF8), Err(MkCharError::EmbeddedNul));
        let s = "\u{3b1}".as_bytes();
        assert_eq!(c.mk_char_ce(s, ce::SYMBOL), Err(MkCharError::UnknownMask(ce::SYMBOL)));
        assert_eq!(c.mk_char_ce(s, ce::ANY), Err(MkCharError::UnknownMask(ce::ANY)));
        // a native string already cached is what CE_SYMBOL and CE_ANY find
        let n = c.mk_char_ce(s, ce::NATIVE).unwrap();
        assert_eq!(c.mk_char_ce(s, ce::SYMBOL), Ok(n));
        assert_eq!(c.mk_char_ce(s, ce::ANY), Ok(n));
        assert_eq!(c.find_char(s, gp::UTF8), None);
    }

    #[test]
    fn translation_needs() {
        let (u, l, b, a, n) = (gp::UTF8, gp::LATIN1, gp::BYTES, gp::ASCII, 0);
        use Translation::*;
        for loc in [UTF8_LOCALE, LATIN1_LOCALE, C_LOCALE] {
            assert_eq!(to_native(b, false, loc), Err(BytesEncoding));
            assert_eq!(to_utf8(b, false, loc), Err(BytesEncoding));
            assert_eq!(to_native(a, false, loc), Ok(None));
            assert_eq!(to_utf8(a, false, loc), Ok(None));
            assert_eq!(to_native(n, false, loc), Ok(None));
            assert_eq!(to_utf8(u, false, loc), Ok(None));
        }
        assert_eq!(to_native(u, false, UTF8_LOCALE), Ok(None));
        assert_eq!(to_native(u, false, LATIN1_LOCALE), Ok(FromUtf8));
        assert_eq!(to_native(u, true, C_LOCALE), Ok(None));
        assert_eq!(to_native(l, false, UTF8_LOCALE), Ok(FromLatin1));
        assert_eq!(to_native(l, false, LATIN1_LOCALE), Ok(None));
        assert_eq!(to_utf8(l, false, UTF8_LOCALE), Ok(FromLatin1));
        assert_eq!(to_utf8(n, false, UTF8_LOCALE), Ok(None));
        assert_eq!(to_utf8(n, false, LATIN1_LOCALE), Ok(FromLatin1));
        assert_eq!(to_utf8(n, false, C_LOCALE), Ok(FromNative));
        assert_eq!(to_utf8(n, true, C_LOCALE), Ok(None));
        assert_eq!(BytesEncoding::MESSAGE, r#"translating strings with "bytes" encoding is not allowed"#);
    }

    #[test]
    fn char_is_utf8_and_latin1() {
        let (u, l, b, a, n) = (gp::UTF8, gp::LATIN1, gp::BYTES, gp::ASCII, 0);
        assert!(is_utf8(a, false, C_LOCALE) && is_utf8(u, false, C_LOCALE));
        assert!(is_utf8(n, false, UTF8_LOCALE) && !is_utf8(n, false, C_LOCALE));
        assert!(!is_utf8(n, true, UTF8_LOCALE) && !is_utf8(l, false, UTF8_LOCALE));
        assert!(!is_utf8(b, false, UTF8_LOCALE));
        assert!(is_latin1(a, false, C_LOCALE) && is_latin1(l, false, C_LOCALE));
        assert!(is_latin1(n, false, LATIN1_LOCALE) && !is_latin1(n, false, UTF8_LOCALE));
        assert!(!is_latin1(u, false, LATIN1_LOCALE) && !is_latin1(b, false, LATIN1_LOCALE));
        assert!(!is_latin1(n, true, LATIN1_LOCALE));
    }
}
