//! String comparison and translation as GNU R does them: `Seql`,
//! `Scollate` (C collation, or ICU's root collator through
//! `robigo_text::collate`), `translateChar` and `translateCharUTF8`, over
//! `robigo-text`'s strings.

use robigo_rtcore::{Cx, Diag, Locale, RError, Root};
use robigo_text::{Enc, Str, TranslateError, translate_char};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::sync::OnceLock;

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

/// `translateChar(STRING_ELT(x, i))`: element `i` (below `x`'s length, the
/// caller's part) of the character vector `x` in the native encoding, or
/// GNU R's error for one it cannot be (a "bytes" string). An `x` that is
/// not a character vector is `STRING_ELT`'s error, a package ALTREP one
/// robigo's "not implemented" error: never a panic.
// R: sysutils.c:translateChar
pub fn native_elt(cx: &mut Cx, x: Root<'_>, i: usize) -> robigo_rtcore::RResult<Vec<u8>> {
    let v = x.bind(cx);
    if v.sexptype() != robigo_rtcore::types::STRSXP {
        let msg = format!(
            "STRING_ELT() can only be applied to a 'character vector', not a '{}'",
            crate::util::type_to_char(v)
        );
        return Err(cx.error(&msg));
    }
    if v.is_package_altrep() {
        return Err(crate::util::not_yet(cx, None, "translateChar() of a package ALTREP vector"));
    }
    let r = {
        let ch = x.bind(cx).as_vector::<robigo_rtcore::Char<'_>>().unwrap().get(i);
        to_native(crate::coerce::text(ch), cx.locale()).map(|b| b.into_owned())
    };
    r.map_err(|e| cx.error(&e.to_string()))
}

/// `installTrChar(c)`'s name: `c` in the native encoding
/// (`translateToNative(mustWork = 2)`: what iconv cannot convert escaped,
/// with the text of the warning GNU R then gives), or the message of the
/// error it raises: a "bytes" string, or (`install`, `installNoTrChar`) a
/// name of no bytes or of more than 10000.
// R: sysutils.c:installTrChar, names.c:installNoTrChar, names.c:install
pub fn install_tr_name(s: Str<'_>, loc: Locale) -> Result<(Cow<'_, [u8]>, Option<String>), String> {
    let (n, w) = robigo_text::translate_char_warn(s, ctype(loc)).map_err(|e| e.to_string())?;
    check_install_name(&n)?;
    Ok((n, w))
}

/// `install(name)`'s checks of a name.
// R: names.c:install, names.c:installNoTrChar
pub fn check_install_name(name: &[u8]) -> Result<(), String> {
    if name.is_empty() {
        return Err("attempt to use zero-length variable name".into());
    }
    if name.len() > MAXIDSIZE {
        return Err(format!("variable names are limited to {MAXIDSIZE} bytes"));
    }
    Ok(())
}

/// GNU R's `MAXIDSIZE`: the longest symbol name, in bytes.
pub const MAXIDSIZE: usize = 10000;

/// `installTrChar(STRING_ELT(c, i))`, rooted in `s`: the symbol named by
/// the native translation of element `i` of the character vector `c`,
/// with GNU R's warning for what could not be translated (given before an
/// error of `install`'s) and its errors. A name the isolate has already is
/// found in place. A package ALTREP `c` is robigo's "not implemented"
/// error (`STRING_ELT` would run its method).
// R: sysutils.c:installTrChar
pub fn install_tr_char<'s>(
    cx: &mut Cx,
    s: robigo_rtcore::Scope<'s>,
    c: Root<'_>,
    i: usize,
) -> robigo_rtcore::RResult<Root<'s>> {
    if c.bind(cx).is_package_altrep() {
        return Err(crate::util::not_yet(cx, None, "installTrChar() of a package ALTREP vector"));
    }
    let r = {
        let cx = &*cx;
        let ch = c.bind(cx).as_vector::<robigo_rtcore::Char<'_>>().unwrap().get(i);
        let t = robigo_text::translate_char_warn(crate::coerce::text(ch), ctype(cx.locale()));
        match t {
            Ok((n, None)) if check_install_name(&n).is_ok() => match cx.find_symbol(&n) {
                Some(y) => return Ok(s.root(cx, y.as_ref())),
                None => Ok((n.into_owned(), None)),
            },
            Ok((n, w)) => Ok((n.into_owned(), w)),
            Err(e) => Err(e.to_string()),
        }
    };
    let (name, w) = r.map_err(|m| cx.error(&m))?;
    if let Some(w) = w {
        cx.warning(&format!("unable to translate '{w}' to native encoding"))?;
    }
    check_install_name(&name).map_err(|m| cx.error(&m))?;
    cx.install_rooted(s, &name)
}

/// `translateCharFP(s)`: `s` in the native encoding, or the message of GNU
/// R's `error()`: a "bytes" string, or what iconv could not convert.
// R: sysutils.c:translateCharFP, translateToNative(mustWork = 1)
pub fn to_native_fp(s: Str<'_>, loc: Locale) -> Result<Cow<'_, [u8]>, String> {
    match robigo_text::translate_char2(s, ctype(loc)) {
        Ok(b) => Ok(b),
        Err(robigo_text::Untranslatable::Bytes) => {
            Err("translating strings with \"bytes\" encoding is not allowed".to_owned())
        }
        Err(robigo_text::Untranslatable::Failed(m)) => Err(format!("unable to translate '{m}' to native encoding")),
    }
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
    /// A string ICU's root collator would read without normalizing, which
    /// ICU4X cannot (not in FCD form; robigo_text::collate).
    NotFcd,
    /// `R_ICU_LOCALE` names an ICU locale: robigo has the root collation only.
    IcuLocale,
    /// The environment's `LC_COLLATE` is a locale whose ICU collation may
    /// tailor the root one (not C, `C.UTF-8` or English).
    EnvLocale,
}

impl CollateError {
    /// The error GNU R raises (`Bytes`), or what robigo has not implemented
    /// (reported against `call`, as `not_yet`).
    pub fn diag<'r>(self, call: Option<Root<'r>>) -> Diag<'r> {
        let what = match self {
            // translateChar's error(), against the context
            CollateError::Bytes => return Diag::error("translating strings with \"bytes\" encoding is not allowed"),
            CollateError::NotFcd => {
                "ordering strings not in FCD form (ICU collates them unnormalized, ICU4X only normalized)"
            }
            CollateError::IcuLocale => "ICU collation in the locale R_ICU_LOCALE names",
            CollateError::EnvLocale => {
                "ICU collation in the locale LC_COLLATE names (other than C, C.UTF-8 and English)"
            }
        };
        Diag::error_call(call, format!("robigo: {what} is not implemented yet"))
    }

    /// [`CollateError::diag`], raised.
    pub fn raise(self, cx: &mut Cx, call: Option<Root<'_>>) -> RError {
        cx.raise(self.diag(call))
    }
}

/// What `Scollate` compares with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Collation {
    /// `strcoll` of the native translations (`collator == NULL`): `strcmp`
    /// in the locales robigo models (`C`, and `C.UTF-8`'s code point order).
    C,
    /// ICU's root collator, `C.UTF-8`'s.
    Root,
    /// ICU's collator of the locale `R_ICU_LOCALE` names.
    Other,
    /// ICU's collator of the environment's `LC_COLLATE`, a locale that may
    /// tailor the root collation.
    OtherEnv,
}

/// The collation `Scollate` sets up at its first call (util.c): C when
/// `LC_ALL` (or, unset or empty, `LC_COLLATE`) is "C" in the environment,
/// or when `getLocale()` (`R_ICU_LOCALE` when set and not empty, else
/// `setlocale(LC_COLLATE)`, the isolate's) is "C"; ICU's collator for that
/// locale otherwise. GNU R reads the environment once a process (until
/// `Sys.setlocale`, which robigo-rt does not have).
// R: util.c:Scollate, getLocale
fn collation(loc: Locale) -> Collation {
    static ENV: OnceLock<(bool, Option<Vec<u8>>, bool)> = OnceLock::new();
    let (use_c, icu, root) = ENV.get_or_init(|| {
        let get = |n: &str| std::env::var_os(n).filter(|v| !v.is_empty());
        let envl = get("LC_ALL").or_else(|| std::env::var_os("LC_COLLATE"));
        let use_c = envl.is_some_and(|v| v.as_encoded_bytes() == b"C");
        // setlocale(LC_COLLATE, NULL) as startup set it (LC_ALL, else
        // LC_COLLATE, else LANG): the collations robigo has are root's,
        // C.UTF-8's and English's (ICU's "en" has no tailoring)
        let name = get("LC_ALL").or_else(|| get("LC_COLLATE")).or_else(|| get("LANG"));
        let root = name.is_none_or(|v| {
            let b = v.as_encoded_bytes();
            b.starts_with(b"C.")
                || b == b"C"
                || b == b"POSIX"
                || b == b"en"
                || b.starts_with(b"en_")
                || b.starts_with(b"en.")
        });
        (use_c, get("R_ICU_LOCALE").map(|v| v.as_encoded_bytes().to_vec()), root)
    });
    match icu {
        _ if *use_c => Collation::C,
        Some(l) if l.as_slice() == b"C" => Collation::C,
        Some(_) => Collation::Other,
        None if loc.collate_c => Collation::C,
        None if *root => Collation::Root,
        None => Collation::OtherEnv,
    }
}

/// GNU R's `Scollate(a, b)`: `strcoll` of the native translations under C
/// collation, else ICU's root collator over the UTF-8 translations.
// R: util.c:Scollate
pub fn scollate(a: Str<'_>, b: Str<'_>, loc: Locale) -> Result<Ordering, CollateError> {
    match collation(loc) {
        Collation::C => {
            let p = to_native(a, loc).map_err(|_| CollateError::Bytes)?;
            let q = to_native(b, loc).map_err(|_| CollateError::Bytes)?;
            Ok(p.as_ref().cmp(q.as_ref()))
        }
        Collation::Root => {
            let p = to_utf8(a, loc).map_err(|_| CollateError::Bytes)?;
            let q = to_utf8(b, loc).map_err(|_| CollateError::Bytes)?;
            robigo_text::collate::root_strcoll(&p, &q).map_err(|_| CollateError::NotFcd)
        }
        Collation::Other => Err(CollateError::IcuLocale),
        Collation::OtherEnv => Err(CollateError::EnvLocale),
    }
}
