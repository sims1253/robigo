//! grep.c: `strsplit`, `grep`/`grepl`, `sub`/`gsub`, `regexpr`/`gregexpr`
//! and `regexec`, over TRE (`robigo_text::tre`, `perl = FALSE`) and the
//! system's PCRE2 (`perl = TRUE`).
//!
//! A builtin runs in three phases, so that what it reports comes in GNU
//! R's order while nothing is allocated until the results are known:
//!
//! 1. its arguments are checked under `&mut Cx` (GNU R's errors and
//!    warnings before the loop);
//! 2. the matching runs over borrowed strings, recording the warnings GNU R
//!    gives as it goes ([`Report`]) and ending at the first error;
//! 3. the warnings are given, in order, then the error, or the result is
//!    built.
//!
//! A warning that a handler turns into an exit ends the builtin there, as it
//! would in GNU R: what phase 2 did after it is never seen.

// named after its C function (do_grep), like its siblings
#[allow(clippy::module_inception)]
pub mod grep;
pub mod regexpr;
pub mod strsplit;
pub mod sub;

pub use grep::do_grep;
pub use regexpr::{do_regexec, do_regexpr};
pub use strsplit::do_strsplit;
pub use sub::do_gsub;

use crate::coerce::text;
use crate::text::ctype;
use crate::util::not_yet;
use robigo_rtcore::{Char, Cx, Encoding, Locale, RError, Root};
use robigo_text::tre::{self, Match, Regex};
use robigo_text::{Untranslatable, WideError};
use std::borrow::Cow;
use std::sync::OnceLock;

/// grep.c's `NWARN`: at most this many "input string %d is invalid"
/// warnings per call.
pub(crate) const NWARN: usize = 5;

/// Why phase 2 stopped: GNU R's `error(...)` (reported against the
/// calling closure), or a path robigo does not have yet.
#[derive(Debug)]
pub(crate) enum Fail {
    Error(String),
    NotYet(String),
}

/// What phase 2 reports: the warnings so far, in order.
#[derive(Default)]
pub(crate) struct Report {
    warnings: Vec<String>,
    /// `nwarn` of the "input string is invalid" warnings.
    nwarn: usize,
}

impl Report {
    pub(crate) fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }

    /// `if (nwarn++ < NWARN) warning(msg)`.
    pub(crate) fn warn_capped(&mut self, msg: impl FnOnce() -> String) {
        if self.nwarn < NWARN {
            self.warnings.push(msg());
        }
        self.nwarn += 1;
    }

    /// Phase 3's start: the warnings, then the error.
    pub(crate) fn finish<T>(self, cx: &mut Cx, call: Root<'_>, r: Result<T, Fail>) -> Result<T, RError> {
        for w in &self.warnings {
            // R: errors.c:warning drops one trailing newline
            cx.warning(w.strip_suffix('\n').unwrap_or(w))?;
        }
        match r {
            Ok(t) => Ok(t),
            Err(Fail::Error(m)) => Err(cx.error(&m)),
            Err(Fail::NotYet(what)) => Err(not_yet(cx, Some(call), &what)),
        }
    }
}

/// A string of a result: NA, or the bytes of a CHARSXP and its mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Elt {
    Na,
    Str(Vec<u8>, Encoding),
}

impl Elt {
    /// The CHARSXP.
    pub(crate) fn make<'g>(&self, cx: &'g Cx) -> Result<Char<'g>, robigo_rtcore::NeedGc> {
        match self {
            Elt::Na => Ok(cx.na_string()),
            Elt::Str(b, e) => cx.mk_char(b, *e),
        }
    }

    /// The same CHARSXP as `c`.
    pub(crate) fn of(cx: &Cx, c: Char<'_>) -> Elt {
        if c == cx.na_string() { Elt::Na } else { Elt::Str(c.bytes().to_vec(), c.enc()) }
    }
}

// ---- the encodings of the inputs ---------------------------------------

/// The strings of a character vector.
pub(crate) fn strings<'g>(x: robigo_rtcore::Ref<'g>) -> robigo_rtcore::Vector<'g, Char<'g>> {
    x.as_vector::<Char<'g>>().expect("robigo: a character vector")
}

// R: grep.c:only_ascii
pub(crate) fn only_ascii(cx: &Cx, x: &[Char<'_>]) -> bool {
    x.iter().all(|&c| c.is_ascii() || c == cx.na_string())
}

/// `have_bytes`, `have_utf8`, `have_latin1`: some string is marked `enc`.
// R: grep.c:have_bytes, have_utf8, have_latin1
pub(crate) fn have(x: &[Char<'_>], enc: Encoding) -> bool {
    x.iter().any(|c| c.enc() == enc)
}

// ---- the marks of the results -------------------------------------------

/// `StringTrue(getenv(name))`, read once as GNU R does.
fn env_true(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| matches!(v.as_str(), "T" | "True" | "TRUE" | "true"))
}

fn mark_old_as_bytes() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    *F.get_or_init(|| env_true("_R_REGEX_MARK_OLD_RESULT_AS_BYTES_"))
}

fn mark_new_as_bytes() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    *F.get_or_init(|| env_true("_R_REGEX_MARK_NEW_RESULT_AS_BYTES_"))
}

/// `markBytesOld(x, useBytes, haveBytesInput)`: an input string that was
/// not changed, as it is unless `_R_REGEX_MARK_OLD_RESULT_AS_BYTES_` says
/// to mark it "bytes".
// R: grep.c:markBytesOld
pub(crate) fn mark_bytes_old(cx: &Cx, x: Char<'_>, use_bytes: bool, have_bytes_input: bool) -> Elt {
    if !mark_old_as_bytes()
        || !have_bytes_input
        || !use_bytes
        || x.is_ascii()
        || x.enc() == Encoding::Bytes
        || x == cx.na_string()
    {
        Elt::of(cx, x)
    } else {
        Elt::Str(x.bytes().to_vec(), Encoding::Bytes)
    }
}

/// `mkBytesNew(s, haveBytesInput)`: the mark of a string made with
/// `useBytes`.
// R: grep.c:mkBytesNew
pub(crate) fn bytes_new(have_bytes_input: bool) -> Encoding {
    if have_bytes_input || mark_new_as_bytes() { Encoding::Bytes } else { Encoding::Native }
}

/// `markKnown(s, ref)`: UTF-8 when `ref` is marked latin1 or UTF-8 and the
/// locale is known to be UTF-8, else native.
// R: util.c:markKnown
pub(crate) fn mark_known(r: Char<'_>, loc: Locale) -> Encoding {
    if matches!(r.enc(), Encoding::Utf8 | Encoding::Latin1) && loc.utf8 { Encoding::Utf8 } else { Encoding::Native }
}

/// `mkCharWLenASCII(wc, nc, maybe_ascii)` with `wc` NUL-terminated after
/// its last element: UTF-8. As in the C, only `wc[..nc]` is read when it
/// is all ASCII and `maybe_ascii`; otherwise `wcstoutf8` converts what of
/// `wc` fits in `(nc + 1) * 4` bytes with the terminator (all of it beyond
/// 8192).
// R: grep.c:mkCharWLenASCII, util.c:wcstoutf8, util.c:Rwcrtomb32
pub(crate) fn wide_utf8(wc: &[i32], nc: usize, maybe_ascii: bool) -> Vec<u8> {
    if maybe_ascii && wc[..nc].iter().all(|&u| (u as u32) <= 127) {
        return wc[..nc].iter().map(|&u| u as u8).collect();
    }
    let nb = (nc + 1) * 4;
    let limit = if nb <= 8192 { nb } else { usize::MAX };
    let mut out = Vec::with_capacity(nb.min(4 * wc.len() + 1));
    for &u in wc {
        let c = u as u32;
        // utf8_table1: the length of the UTF-8 form, up to 6 bytes
        let n = match c {
            0..=0x7f => 1,
            0x80..=0x7ff => 2,
            0x800..=0xffff => 3,
            0x1_0000..=0x1f_ffff => 4,
            0x20_0000..=0x3ff_ffff => 5,
            _ => 6,
        };
        // need space for the terminal null
        if out.len() + n > limit.saturating_sub(1) {
            break;
        }
        match char::from_u32(c) {
            Some(ch) => out.extend_from_slice(ch.encode_utf8(&mut [0u8; 4]).as_bytes()),
            None => {
                // a surrogate or beyond U+10FFFF, which no conversion here
                // gives: Rwcrtomb32's bytes
                debug_assert!(false, "robigo: wide character {c:#x} from a conversion");
                let mut buf = [0u8; 6];
                let mut v = c;
                for j in (1..n).rev() {
                    buf[j] = 0x80 | (v & 0x3f) as u8;
                    v >>= 6;
                }
                const TABLE2: [u8; 6] = [0, 0xc0, 0xe0, 0xf0, 0xf8, 0xfc];
                buf[0] = TABLE2[n - 1] | v as u8;
                out.extend_from_slice(&buf[..n]);
            }
        }
    }
    out
}

// ---- validity and translation ---------------------------------------------

/// `utf8clen(c)`: the length of the UTF-8 character `c` starts (1 for a
/// byte that starts none).
// R: util.c:utf8clen
#[inline]
pub(crate) fn utf8clen(c: u8) -> usize {
    if c & 0xc0 != 0xc0 {
        return 1;
    }
    const T4: [u8; 64] = [
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2,
        2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5,
    ];
    1 + T4[(c & 0x3f) as usize] as usize
}

/// `utf8Valid(s)`.
// R: util.c:utf8Valid, valid_utf8.h
#[inline]
pub(crate) fn utf8_valid(b: &[u8]) -> bool {
    std::str::from_utf8(b).is_ok()
}

/// `mbcsValid(s)` in a UTF-8 locale: glibc's `mbstowcs` takes all of it.
// R: util.c:mbcsValid
pub(crate) fn mbcs_valid(b: &[u8]) -> bool {
    let mut i = 0;
    loop {
        match robigo_text::mbrtowc_utf8(b, i) {
            (0, _) => break true,
            (n, _) if n < 0 => break false,
            (n, _) => i += n as usize,
        }
    }
}

/// `trCharUTF82(s)`: `None` after its warning when iconv fails.
// R: sysutils.c:trCharUTF82
pub(crate) fn tr_utf8<'a>(c: Char<'a>, loc: Locale, rep: &mut Report) -> Option<Cow<'a, [u8]>> {
    match robigo_text::translate_char_utf8_2(text(c), ctype(loc)) {
        Ok(b) => Some(b),
        Err(Untranslatable::Failed(m)) => {
            rep.warn(format!("unable to translate '{m}' to UTF-8"));
            None
        }
        Err(Untranslatable::Bytes) => unreachable!("robigo: a \"bytes\" string is matched as bytes"),
    }
}

/// `translateCharFP2(s)`: `None` after its warning when iconv fails.
// R: sysutils.c:translateCharFP2
pub(crate) fn tr_native<'a>(c: Char<'a>, loc: Locale, rep: &mut Report) -> Option<Cow<'a, [u8]>> {
    match robigo_text::translate_char2(text(c), ctype(loc)) {
        Ok(b) => Some(b),
        Err(Untranslatable::Failed(m)) => {
            rep.warn(format!("unable to translate '{m}' to native encoding"));
            None
        }
        Err(Untranslatable::Bytes) => unreachable!("robigo: a \"bytes\" string is matched as bytes"),
    }
}

/// `translateChar(s)`, for messages.
// R: sysutils.c:translateChar
pub(crate) fn tr_char(c: Char<'_>, loc: Locale) -> String {
    match robigo_text::translate_char(text(c), ctype(loc)) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(_) => String::from_utf8_lossy(c.bytes()).into_owned(),
    }
}

/// `wtransChar2(s)`: `None` after its warning when iconv fails.
// R: sysutils.c:wtransChar2
pub(crate) fn wtrans<'a>(c: Char<'a>, loc: Locale, rep: &mut Report) -> Option<Vec<i32>> {
    match robigo_text::wtrans_char2(text(c), ctype(loc)) {
        Ok(w) => Some(w),
        Err(WideError::Invalid(m)) => {
            rep.warn(format!("unable to translate '{m}' to a wide string"));
            None
        }
        Err(WideError::Bytes) => unreachable!("robigo: a \"bytes\" string is matched as bytes"),
    }
}

/// An input string in the encoding the matching uses, or `None` after the
/// warnings GNU R gives for one it cannot use (`NA` in the result).
// R: grep.c (the `useBytes` / `use_UTF8` / native preamble of each loop)
pub(crate) fn input<'a>(
    c: Char<'a>,
    i: usize,
    use_bytes: bool,
    use_utf8: bool,
    loc: Locale,
    rep: &mut Report,
) -> Option<Cow<'a, [u8]>> {
    if use_bytes {
        Some(Cow::Borrowed(c.bytes()))
    } else if use_utf8 {
        let b = tr_utf8(c, loc, rep).filter(|b| utf8_valid(b));
        if b.is_none() {
            rep.warn_capped(|| format!("input string {} is invalid UTF-8", i + 1));
        }
        b
    } else {
        let b = tr_native(c, loc, rep).filter(|b| !loc.utf8 || mbcs_valid(b));
        if b.is_none() {
            rep.warn_capped(|| format!("input string {} is invalid in this locale", i + 1));
        }
        b
    }
}

/// A pattern in the encoding the matching uses, or GNU R's error.
pub(crate) fn pattern<'a>(
    c: Char<'a>,
    what: impl Fn() -> String,
    use_bytes: bool,
    use_utf8: bool,
    loc: Locale,
    rep: &mut Report,
) -> Result<Cow<'a, [u8]>, Fail> {
    if use_bytes {
        Ok(Cow::Borrowed(c.bytes()))
    } else if use_utf8 {
        tr_utf8(c, loc, rep)
            .filter(|b| utf8_valid(b))
            .ok_or_else(|| Fail::Error(format!("{} is invalid UTF-8", what())))
    } else {
        tr_native(c, loc, rep)
            .filter(|b| !loc.utf8 || mbcs_valid(b))
            .ok_or_else(|| Fail::Error(format!("{} is invalid in this locale", what())))
    }
}

// ---- TRE ------------------------------------------------------------------

/// `reg_report(rc, &reg, pat)` for a compilation that failed (`pat` is
/// NULL where GNU R had no narrow pattern), and the errors of TRE's
/// asserts.
// R: grep.c:reg_report
pub(crate) fn tre_fail(e: tre::Error, pat: Option<&str>, rep: &mut Report) -> Fail {
    match e {
        tre::Error::Status(s) => {
            let errbuf = s.message();
            match pat {
                Some(pat) => {
                    // PR#16600 - the regex may be so long that the TRE error
                    // description is truncated out from the message, so give
                    // also a warning
                    rep.warn(format!("TRE pattern compilation error '{errbuf}'"));
                    Fail::Error(format!("invalid regular expression '{pat}', reason '{errbuf}'"))
                }
                None => Fail::Error(format!("invalid regular expression, reason '{errbuf}'")),
            }
        }
        e => exec_fail(e),
    }
}

/// A string as TRE matches it: bytes (`tre_regexecb`, `tre_regexec`) or
/// wide characters (`tre_regwexec`).
pub(crate) enum Text<'a> {
    Bytes(Cow<'a, [u8]>),
    Wide(Vec<i32>),
}

impl Text<'_> {
    pub(crate) fn subj(&self) -> Subj<'_> {
        match self {
            Text::Bytes(b) => Subj::Bytes(b),
            Text::Wide(w) => Subj::Wide(w),
        }
    }
}

/// A borrowed [`Text`].
#[derive(Clone, Copy)]
pub(crate) enum Subj<'a> {
    Bytes(&'a [u8]),
    Wide(&'a [i32]),
}

/// `tre_regexecb`/`tre_regwexec(re, s, nmatch, pm, eflags)`: whether it
/// matched, `Err(status)` for a status GNU R reports (or ignores), and a
/// [`Fail`] for an assert or what robigo's TRE does not have.
pub(crate) fn tre_exec(
    re: &Regex,
    s: Subj<'_>,
    pm: &mut [Match],
    eflags: i32,
) -> Result<Result<bool, tre::Status>, Fail> {
    let r = match s {
        Subj::Bytes(b) => re.exec_bytes(b, pm, eflags),
        Subj::Wide(w) => re.exec_wide(w, pm, eflags),
    };
    match r {
        Ok(m) => Ok(Ok(m)),
        Err(tre::Error::Status(st)) => Ok(Err(st)),
        Err(e) => Err(exec_fail(e)),
    }
}

/// An input string for TRE's wide matching, or `None` after its warnings.
pub(crate) fn wide_input(c: Char<'_>, i: usize, loc: Locale, rep: &mut Report) -> Option<Vec<i32>> {
    let w = wtrans(c, loc, rep);
    if w.is_none() {
        rep.warn_capped(|| format!("input string {} is invalid", i + 1));
    }
    w
}

/// A matcher error other than a status: an `R_assert` (an error), or what
/// robigo's TRE does not have.
pub(crate) fn exec_fail(e: tre::Error) -> Fail {
    match e {
        tre::Error::Unsupported(what) => Fail::NotYet(format!("'{what}'")),
        e => Fail::Error(e.message()),
    }
}

/// `tre_regcomp` of a native pattern: the bytes in the C locale, the
/// UTF-8 characters in a UTF-8 one (the pattern is valid).
pub(crate) fn tre_comp_native(p: &[u8], cflags: i32, loc: Locale) -> Result<Regex, tre::Error> {
    let w: Vec<i32> = if loc.utf8 {
        robigo_text::wtrans_char2(robigo_text::Str::new(p, robigo_text::Enc::Utf8), ctype(loc))
            .expect("robigo: a valid pattern")
    } else {
        p.iter().map(|&b| b as i32).collect()
    };
    Regex::new(&w, cflags, ctype(loc))
}

// ---- fixed ----------------------------------------------------------------

/// `fgrep_one(pat, target, useBytes, use_UTF8, &next)`: where `pat` first
/// occurs in `target`, in characters (bytes with `useBytes`, or in a
/// single-byte locale without `use_UTF8`), and the byte after it.
// R: grep.c:fgrep_one
pub(crate) fn fgrep_one(
    pat: &[u8],
    target: &[u8],
    use_bytes: bool,
    use_utf8: bool,
    loc: Locale,
) -> Option<(usize, usize)> {
    let plen = pat.len();
    if plen == 0 {
        return Some((0, 1));
    }
    let mbcs = loc.utf8;
    if plen == 1 && (use_bytes || !(mbcs || use_utf8)) {
        // a single byte is a common case
        return target.iter().position(|&b| b == pat[0]).map(|i| (i, i + 1));
    }
    if !use_bytes && use_utf8 {
        let ib = find(target, pat)?;
        // count leading UTF-8 bytes
        let i = target[..ib].iter().filter(|&&b| b & 0xc0 != 0x80).count();
        Some((i, ib + plen))
    } else if !use_bytes && mbcs {
        // skip along by chars (the string is valid in the locale)
        let (mut ib, mut i) = (0, 0);
        while ib + plen <= target.len() {
            if target[ib..].starts_with(pat) {
                return Some((i, ib + plen));
            }
            ib += utf8clen(target[ib]);
            i += 1;
        }
        None
    } else {
        find(target, pat).map(|i| (i, i + plen))
    }
}

/// `fgrep_one_bytes(pat, patlen, target, len, useBytes, use_UTF8)`: where
/// `pat` first occurs in `target`, in bytes.
// R: grep.c:fgrep_one_bytes
pub(crate) fn fgrep_one_bytes(
    pat: &[u8],
    target: &[u8],
    use_bytes: bool,
    use_utf8: bool,
    loc: Locale,
) -> Option<usize> {
    let plen = pat.len();
    if plen == 0 {
        return Some(0);
    }
    let mbcs = loc.utf8;
    // UTF-8 allows for byte-based search of valid pattern string inside
    // valid target string, because lead and continuation bytes are taken
    // from distinct ranges.
    if plen == 1 && (use_bytes || use_utf8 || !mbcs) {
        return target.iter().position(|&b| b == pat[0]);
    }
    if !use_utf8 && !use_bytes && mbcs {
        // skip along by chars
        let mut ib = 0;
        while ib + plen <= target.len() {
            if target[ib..].starts_with(pat) {
                return Some(ib);
            }
            ib += utf8clen(target[ib]);
        }
        None
    } else {
        find(target, pat)
    }
}

/// `strstr(hay, needle)`'s offset.
pub(crate) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    let first = needle[0];
    let mut from = 0;
    while let Some(k) = hay[from..=hay.len() - needle.len()].iter().position(|&b| b == first) {
        let at = from + k;
        if hay[at..].starts_with(needle) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

// ---- PCRE -----------------------------------------------------------------

/// Whether `PCRE2_NOTBOL` could change a match of `pattern` from an offset
/// past 0: only a lookbehind can reach back to a `^` at the subject's start.
/// A plain byte search for both, so it never misses one (a `[^a]` or `\^`
/// only costs a not_yet when there is also a lookbehind); parsing PCRE's
/// syntax to be finer is not worth its corners.
fn notbol_matters(p: &[u8]) -> bool {
    // (?<=, (?<!, (?<* and whatever else is not a group name (?<name>
    let lookbehind = p.windows(3).enumerate().any(|(i, w)| {
            w == b"(?<" && !p.get(i + 3).is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_')
        })
        // (*plb:...), (*negative_lookbehind:...), (*naplb:...), ...
        || p.windows(2).enumerate().any(|(i, w)| {
            w == b"(*" && {
                let name = &p[i + 2..];
                let name = &name[..name.iter().position(|&b| b == b':' || b == b')').unwrap_or(name.len())];
                name.ends_with(b"lb") || name.ends_with(b"lookbehind")
            }
        });
    lookbehind && p.contains(&b'^')
}

/// A compiled PCRE2 pattern (`R_pcre2_prepare`).
pub(crate) struct Pcre {
    re: pcre2::bytes::Regex,
    /// Whether `PCRE2_NOTBOL` could change a match from an offset past 0:
    /// only a lookbehind reaches back to a `^` at the subject's start.
    notbol_matters: bool,
}

/// The PCRE2 JIT stack's maximum (`jit_stack_size`), and whether
/// `R_PCRE_JIT_STACK_MAXSIZE` was invalid. GNU R reads it when it first
/// allocates the stack, once.
// R: grep.c:jit_stack_size
fn jit_stack_size() -> (usize, bool) {
    static S: OnceLock<(usize, bool)> = OnceLock::new();
    *S.get_or_init(|| match std::env::var_os("R_PCRE_JIT_STACK_MAXSIZE") {
        None => (64 * 1024 * 1024, false),
        Some(p) => {
            let p = p.to_string_lossy();
            let (x, _) = robigo_text::strtod::r_strtod(p.as_bytes());
            if (0.0..=1000.0).contains(&x) {
                ((x * 1024.0 * 1024.0) as i32 as usize, false)
            } else {
                (64 * 1024 * 1024, true)
            }
        }
    })
}

/// PCRE2's `pcre2_get_error_message` text of a crate error.
fn pcre_message(e: &pcre2::Error) -> String {
    let s = e.to_string();
    // the crate's Display: "PCRE2: error <what>: <message>", with
    // " at offset N" in a compile error's <what>
    match s.split_once(": ").and_then(|(_, rest)| rest.split_once(": ")) {
        Some((_, m)) => m.to_string(),
        None => s,
    }
}

impl Pcre {
    /// `R_pcre2_prepare(pattern, subject, use_UTF8, caseless, ...)`: the
    /// pattern compiled, or GNU R's warning and error.
    // R: grep.c:R_pcre2_prepare
    pub(crate) fn prepare(
        pattern: &[u8],
        use_utf8: bool,
        caseless: bool,
        loc: Locale,
        rep: &mut Report,
    ) -> Result<Pcre, Fail> {
        let Ok(pat) = std::str::from_utf8(pattern) else {
            return Err(Fail::NotYet("a PCRE pattern that is not valid UTF-8".into()));
        };
        let (stack, invalid) = jit_stack_size();
        let build = |jit: bool| {
            let mut b = pcre2::bytes::RegexBuilder::new();
            b.utf(use_utf8).caseless(caseless);
            if jit {
                b.jit(true).max_jit_stack_size(Some(stack));
            }
            b.build(pat)
        };
        let re = match build(true) {
            Ok(re) => {
                if invalid {
                    rep.warn("R_PCRE_JIT_STACK_MAXSIZE invalid and ignored");
                }
                re
            }
            Err(e) if matches!(e.kind(), pcre2::ErrorKind::JIT) => {
                // PCRE2_ERROR_JIT_BADOPTION: no JIT in this PCRE2
                if e.code() != -45 {
                    rep.warn(format!("PCRE JIT compilation error\n\t'{}'", pcre_message(&e)));
                }
                build(false).expect("robigo: compiled once")
            }
            Err(e) => {
                let off = e.offset().unwrap_or(0).min(pattern.len());
                rep.warn(format!(
                    "PCRE pattern compilation error\n\t'{}'\n\tat '{}'\n",
                    pcre_message(&e),
                    pcre_native(&pattern[off..], use_utf8, loc)
                ));
                return Err(Fail::Error(format!(
                    "invalid regular expression '{}'",
                    pcre_native(pattern, use_utf8, loc)
                )));
            }
        };
        Ok(Pcre { re, notbol_matters: notbol_matters(pattern) })
    }

    /// `capture_names`: each group's name from PCRE's name table, "" for
    /// one without.
    pub(crate) fn capture_names(&self) -> Vec<String> {
        self.re.capture_names().iter().skip(1).map(|n| n.clone().unwrap_or_default()).collect()
    }

    /// An empty set of group offsets for [`Pcre::match_at`].
    pub(crate) fn locations(&self) -> pcre2::bytes::CaptureLocations {
        self.re.capture_locations()
    }

    /// `pcre2_match(re, subject, len, start, notbol ? PCRE2_NOTBOL : 0,
    /// ...)`: whether it matched (the offsets in `locs`), `Err` a matching
    /// error's message. The `pcre2` crate passes no options, so a pattern
    /// that `PCRE2_NOTBOL` could change is not matched with it.
    pub(crate) fn match_at(
        &self,
        locs: &mut pcre2::bytes::CaptureLocations,
        subject: &[u8],
        start: usize,
        notbol: bool,
    ) -> Result<Result<bool, String>, Fail> {
        if notbol && self.notbol_matters {
            return Err(Fail::NotYet("PCRE2_NOTBOL with a lookbehind and '^'".into()));
        }
        Ok(self.re.captures_read_at(locs, subject, start).map(|m| m.is_some()).map_err(|e| pcre_message(&e)))
    }

    /// `pcre2_match(re, subject, len, 0, 0, ...) >= 0`, `Err` a matching
    /// error's message.
    pub(crate) fn is_match(&self, subject: &[u8]) -> Result<bool, String> {
        self.re.is_match(subject).map_err(|e| pcre_message(&e))
    }

    /// `pcre2_match(re, subject, len, start, 0, ...)`: the match's offsets,
    /// `Ok(None)` for no match, `Err` a matching error's message.
    pub(crate) fn find_at(&self, subject: &[u8], start: usize) -> Result<Option<(usize, usize)>, String> {
        match self.re.find_at(subject, start) {
            Ok(m) => Ok(m.map(|m| (m.start(), m.end()))),
            Err(e) => Err(pcre_message(&e)),
        }
    }
}

/// `R_pcre_exec_error(rc, i)`'s warning for a matching error.
// R: grep.c:R_pcre_exec_error
pub(crate) fn pcre_exec_error(msg: &str, i: usize) -> String {
    let mut buf = msg.to_string();
    if buf == "recursion limit exceeded" {
        buf.push_str(": consider increasing the C stack size for the R process");
    }
    format!("PCRE error\n\t'{buf}'\n\tfor element {}", i + 1)
}

/// grep.c's `to_native(str, use_UTF8)`: `reEnc(str, CE_UTF8, CE_NATIVE, 1)`
/// for UTF-8 (what the native encoding lacks as `<xx>`).
// R: grep.c:to_native
fn pcre_native(s: &[u8], use_utf8: bool, loc: Locale) -> String {
    if use_utf8 && !loc.utf8 {
        let mut out = String::new();
        for &b in s {
            if b < 0x80 {
                out.push(b as char);
            } else {
                out.push_str(&format!("<{b:02x}>"));
            }
        }
        out
    } else {
        String::from_utf8_lossy(s).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::notbol_matters;

    #[test]
    fn notbol_needs_a_lookbehind_and_an_anchor() {
        // a named group or a negated class is not a lookbehind
        for p in [&b"(?<x>[^a])"[..], b"(?<=a)b", b"^a|(*atomic:b)"] {
            assert!(!notbol_matters(p), "{}", String::from_utf8_lossy(p));
        }
        // escapes that a lexer could misread hide nothing from a byte search
        for p in [
            &b"(?<=^)a"[..],
            b"(?<!^a)b",
            b"(?m)a|(?<*^a)b",
            b"^|(*plb:x)",
            b"(*negative_lookbehind:^)b",
            b"\\c[|a|(?<=^a)b",
            b"\\cA{|a|(?<=^a)b",
            b"[\\c[:]|(?<=^a)b",
        ] {
            assert!(notbol_matters(p), "{}", String::from_utf8_lossy(p));
        }
    }
}
