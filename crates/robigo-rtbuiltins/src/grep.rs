//! grep.c's `grep`, `grepl`, `sub` and `gsub`: `do_grep` and `do_gsub`
//! ported (the argument checks, warnings and messages, the encoding
//! decisions and translations, NA handling, the results' encodings and
//! attributes), with `fixed = TRUE` exact and an interim matcher for
//! regular expressions.
//!
//! INTERIM(RX): the matcher ([`ere`]) stands where GNU R calls TRE
//! (`tre_regcompb`/`tre_regwcomp`, `tre_regexecb`/`tre_regwexec`). It
//! knows the part of POSIX extended regular expressions whose meaning in
//! TRE is plain: literals, a backslash before punctuation, `.`, `^`, `$`,
//! bracket expressions of characters and ranges, and `*`, `+` and `?`
//! after one of them. It finds POSIX's leftmost-longest match directly
//! (each start from the left, the longest end the NFA reaches from it), so
//! its matches are TRE's. Anything else (alternation, groups,
//! back-references, intervals, character classes, the escapes TRE gives a
//! meaning, minimal repetition, an ill-formed pattern, `ignore.case`,
//! `perl = TRUE`) raises "not implemented yet (INTERIM(RX))", never a
//! guessed match. RX's TRE port replaces [`ere`]; the callers stay.

use crate::attrib::{self, Attr, Attrs};
use crate::util::as_bool2;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Lgl, Out, RError, RResult, RSym, Ref, Root};
use std::sync::OnceLock;

const NA_INTEGER: i32 = i32::MIN;

/// grep.c's `NWARN`: the most "input string %d is invalid" warnings one
/// call gives.
const NWARN: usize = 5;

/// The interim's refusal of what it does not know.
fn not_rx(cx: &mut Cx, what: &str) -> RError {
    cx.error(&format!("robigo: {what} is not implemented yet (INTERIM(RX))"))
}

/// The interim regular-expression matcher: a pattern of single-unit atoms,
/// each once, optional or repeated, matched leftmost-longest. Units are
/// bytes (GNU R's byte TRE) or code points (its wide-character TRE).
/// INTERIM(RX): replaced by RX's TRE port.
pub mod ere {
    /// What one atom matches.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Atom {
        /// One unit.
        Unit(u32),
        /// `.`: any unit (TRE without `REG_NEWLINE`: a newline too).
        Any,
        /// A bracket expression, as the inclusive ranges TRE builds for it
        /// (a negated one complemented as `tre_parse_bracket` does).
        Set(Vec<(u32, u32)>),
        /// `^`: the start of the string (not with `REG_NOTBOL`).
        Bol,
        /// `$`: the end of the string.
        Eol,
    }

    impl Atom {
        fn matches(&self, c: u32) -> bool {
            match self {
                Atom::Unit(u) => *u == c,
                Atom::Any => true,
                Atom::Set(ranges) => ranges.iter().any(|&(a, b)| a <= c && c <= b),
                Atom::Bol | Atom::Eol => false,
            }
        }

        fn zero_width(&self) -> bool {
            matches!(self, Atom::Bol | Atom::Eol)
        }
    }

    /// How often an atom matches: once, at most once, any number of times.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Times {
        One,
        Opt,
        Star,
    }

    /// A compiled pattern.
    #[derive(Debug, Clone)]
    pub struct Re {
        items: Vec<(Atom, Times)>,
    }

    /// What the matcher does not know, for the caller's error.
    pub type Unsupported = &'static str;

    const fn ch(c: char) -> u32 {
        c as u32
    }

    /// Compile `p`'s units (`REG_EXTENDED`); `Err` names the construct the
    /// matcher does not know.
    // R: tre-parse.c:tre_parse (the subset)
    pub fn compile(p: &[u32]) -> Result<Re, Unsupported> {
        let mut items: Vec<(Atom, Times)> = Vec::new();
        let mut i = 0;
        while i < p.len() {
            let c = p[i];
            let atom = if c == ch('\\') {
                let Some(&e) = p.get(i + 1) else {
                    return Err("a regular expression ending in a backslash");
                };
                i += 1;
                // TRE gives letters and digits meanings (\w, \d, \b, \x,
                // back-references, ...), and \< and \>
                if char::from_u32(e).is_none_or(|e| e.is_ascii_alphanumeric() || e == '<' || e == '>') {
                    return Err("this backslash escape in a regular expression");
                }
                Atom::Unit(e)
            } else if c == ch('.') {
                Atom::Any
            } else if c == ch('^') {
                Atom::Bol
            } else if c == ch('$') {
                Atom::Eol
            } else if c == ch('[') {
                let (set, next) = bracket(p, i + 1)?;
                i = next - 1;
                set
            } else if [ch('|'), ch('('), ch(')')].contains(&c) {
                return Err("alternation or a group in a regular expression");
            } else if c == ch('{') {
                return Err("an interval ({m,n}) in a regular expression");
            } else if [ch('*'), ch('+'), ch('?')].contains(&c) {
                return Err("a repetition operator with nothing to repeat in a regular expression");
            } else {
                Atom::Unit(c)
            };
            i += 1;
            let times = match p.get(i).copied() {
                Some(q) if q == ch('*') => Some(Times::Star),
                Some(q) if q == ch('?') => Some(Times::Opt),
                Some(q) if q == ch('+') => Some(Times::One),
                Some(q) if q == ch('{') => return Err("an interval ({m,n}) in a regular expression"),
                _ => None,
            };
            let Some(t) = times else {
                items.push((atom, Times::One));
                continue;
            };
            if atom.zero_width() {
                return Err("a repetition of an anchor in a regular expression");
            }
            let plus = p[i] == ch('+');
            i += 1;
            // `*?` and the like are TRE's minimal repetition, `**` an error
            if p.get(i).is_some_and(|&n| [ch('*'), ch('+'), ch('?'), ch('{')].contains(&n)) {
                return Err("minimal or repeated repetition operators in a regular expression");
            }
            if plus {
                // x+ is x x*
                items.push((atom.clone(), Times::One));
                items.push((atom, Times::Star));
            } else {
                items.push((atom, t));
            }
        }
        Ok(Re { items })
    }

    /// The bracket expression whose `[` is just before `p[i]`: its set and
    /// the index past its `]`. TRE reads a backslash in brackets as itself.
    // R: tre-parse.c:tre_parse_bracket, tre_parse_bracket_items
    fn bracket(p: &[u32], mut i: usize) -> Result<(Atom, usize), Unsupported> {
        let negate = p.get(i) == Some(&ch('^'));
        if negate {
            i += 1;
        }
        let start = i;
        let mut items: Vec<(u32, u32)> = Vec::new();
        loop {
            let Some(&c) = p.get(i) else {
                return Err("an unterminated bracket expression in a regular expression");
            };
            if c == ch(']') && i > start {
                i += 1;
                break;
            }
            if i + 2 < p.len() && p[i + 1] == ch('-') && p[i + 2] != ch(']') {
                let (min, max) = (c, p[i + 2]);
                if min > max {
                    return Err("an invalid range in a bracket expression");
                }
                items.push((min, max));
                i += 3;
            } else if c == ch('[') && p.get(i + 1).is_some_and(|&n| [ch('.'), ch('='), ch(':')].contains(&n)) {
                return Err("a character class, equivalence class or collating symbol in a regular expression");
            } else {
                if c == ch('-') && p.get(i + 1) != Some(&ch(']')) && i != start {
                    // two ranges may not share an endpoint
                    return Err("an invalid range in a bracket expression");
                }
                items.push((c, c));
                i += 1;
            }
        }
        if !negate {
            return Ok((Atom::Set(items), i));
        }
        // the complement as TRE computes it, over the items sorted by
        // their first character (where two items start at the same
        // character, qsort's order would decide the result)
        items.sort_by_key(|&(min, _)| min);
        if items.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err("a negated bracket expression with two items starting at the same character");
        }
        let (mut curr_min, mut curr_max) = (0i64, 0i64);
        let mut set = Vec::new();
        for (min, max) in items {
            let (min, max) = (min as i64, max as i64);
            if min < curr_max {
                // overlap (TRE keeps curr_min)
                curr_max = curr_max.max(max + 1);
            } else {
                curr_max = min - 1;
                if curr_max >= curr_min {
                    set.push((curr_min as u32, curr_max as u32));
                }
                curr_min = max + 1;
                curr_max = max + 1;
            }
        }
        set.push((curr_min as u32, u32::MAX));
        Ok((Atom::Set(set), i))
    }

    /// A set of NFA states: item indices, `items.len()` the accepting one.
    struct States {
        on: Vec<bool>,
        list: Vec<usize>,
    }

    impl States {
        fn new(n: usize) -> States {
            States { on: vec![false; n + 1], list: Vec::new() }
        }
        fn add(&mut self, i: usize) -> bool {
            if self.on[i] {
                return false;
            }
            self.on[i] = true;
            self.list.push(i);
            true
        }
        fn clear(&mut self) {
            for &i in &self.list {
                self.on[i] = false;
            }
            self.list.clear();
        }
    }

    impl Re {
        /// Add state `i` and what it reaches at position `pos` without
        /// consuming a unit: past an optional or repeated atom, past an
        /// anchor that holds there.
        fn close(&self, st: &mut States, i: usize, pos: usize, len: usize, notbol: bool) {
            let mut todo = vec![i];
            while let Some(k) = todo.pop() {
                if !st.add(k) || k == self.items.len() {
                    continue;
                }
                let (a, t) = &self.items[k];
                let pass = match a {
                    Atom::Bol => pos == 0 && !notbol,
                    Atom::Eol => pos == len,
                    _ => *t != Times::One,
                };
                if pass {
                    todo.push(k + 1);
                }
            }
        }

        /// The leftmost-longest match in `s` starting at or after `from`:
        /// its start and end. `notbol`: `^` does not match at position 0
        /// (`REG_NOTBOL`).
        pub fn find(&self, s: &[u32], from: usize, notbol: bool) -> Option<(usize, usize)> {
            let n = self.items.len();
            let len = s.len();
            let (mut cur, mut next) = (States::new(n), States::new(n));
            for start in from..=len {
                cur.clear();
                self.close(&mut cur, 0, start, len, notbol);
                let mut best = cur.on[n].then_some(start);
                let mut pos = start;
                while pos < len && !cur.list.is_empty() {
                    next.clear();
                    let c = s[pos];
                    for k in 0..cur.list.len() {
                        let i = cur.list[k];
                        if i == n {
                            continue;
                        }
                        let (a, t) = &self.items[i];
                        if a.matches(c) {
                            let to = if *t == Times::Star { i } else { i + 1 };
                            self.close(&mut next, to, pos + 1, len, notbol);
                        }
                    }
                    std::mem::swap(&mut cur, &mut next);
                    pos += 1;
                    if cur.on[n] {
                        best = Some(pos);
                    }
                }
                if let Some(end) = best {
                    return Some((start, end));
                }
            }
            None
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn u(s: &str) -> Vec<u32> {
            s.chars().map(|c| c as u32).collect()
        }
        fn find(p: &str, s: &str) -> Option<(usize, usize)> {
            compile(&u(p)).unwrap().find(&u(s), 0, false)
        }

        #[test]
        fn leftmost_longest() {
            assert_eq!(find("a*", "baaa"), Some((0, 0)));
            assert_eq!(find("a+", "baaa"), Some((1, 4)));
            assert_eq!(find("a*ab", "aaab"), Some((0, 4)));
            assert_eq!(find("x?y", "zxy"), Some((1, 3)));
            assert_eq!(find("^\\.\\.[0-9]+$", "..12"), Some((0, 4)));
            assert_eq!(find("^\\.\\.[0-9]+$", "..1a"), None);
            assert_eq!(find("[^a-c]", "abcd"), Some((3, 4)));
            assert_eq!(find("[]a]", "x]"), Some((1, 2)));
            assert_eq!(find("[a-]", "x-"), Some((1, 2)));
            assert_eq!(find("[\\d]", "5\\"), Some((1, 2)));
            assert_eq!(find("$", "ab"), Some((2, 2)));
            assert_eq!(find("", "ab"), Some((0, 0)));
            assert_eq!(find("b$", "abab"), Some((3, 4)));
            assert_eq!(find(".*", "ab"), Some((0, 2)));
            assert_eq!(find("a^b", "ab"), None);
        }

        #[test]
        fn tre_negation() {
            // TRE's complement of overlapping items keeps d and e
            assert_eq!(find("[^a-cb-e]", "bd"), Some((1, 2)));
            assert_eq!(find("[^a-cb-e]", "abcf"), Some((3, 4)));
        }

        #[test]
        fn refusals() {
            for p in [
                "a|b",
                "(a)",
                "a{2}",
                "[[:alpha:]]",
                "\\w",
                "\\1",
                "\\<",
                "*a",
                "a**",
                "a*?",
                "[a",
                "^*",
                "a\\",
                "[b-a]",
                "[a-c-e]",
                "[^aa-c]",
            ] {
                assert!(compile(&u(p)).is_err(), "{p}");
            }
        }
    }
}

/// How the strings are matched, as `do_grep` and `do_gsub` decide it:
/// `useBytes` (given, everything ASCII, or input marked "bytes"), `use_WC`
/// (TRE on wide characters), `use_UTF8` (fixed or PCRE in UTF-8), or the
/// native encoding (fixed, or TRE outside a multibyte locale).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Bytes,
    Wide,
    Utf8,
    Native,
}

impl Mode {
    /// Units are code points (else bytes).
    fn wide(self) -> bool {
        matches!(self, Mode::Wide | Mode::Utf8)
    }
}

/// The strings `do_grep`/`do_gsub` look at to decide the mode: the first
/// element of the pattern (and of the replacement), all of the text.
fn strings<'a, 'g: 'a>(xs: &'a [(Ref<'g>, usize)]) -> impl Iterator<Item = Char<'g>> + 'a {
    xs.iter().flat_map(|&(x, n)| {
        let v = x.as_vector::<Char<'g>>().unwrap();
        (0..n.min(v.len())).map(move |i| v.get(i))
    })
}

/// The mode and `haveBytesInput`.
// R: grep.c:do_gsub (only_ascii, have_bytes, have_utf8, have_latin1)
fn decide(cx: &Cx, xs: &[(Ref<'_>, usize)], use_bytes: bool, fixed: bool, perl: bool) -> (Mode, bool) {
    let na = cx.na_string();
    let have_bytes = strings(xs).any(|c| c.enc() == Encoding::Bytes);
    let mut use_bytes = use_bytes || strings(xs).all(|c| c.is_ascii() || c == na);
    use_bytes = use_bytes || have_bytes;
    let mut use_utf8 = false;
    if !use_bytes {
        let mbcs = cx.locale().utf8;
        if !fixed && mbcs {
            use_utf8 = true;
        }
        if !use_utf8 {
            use_utf8 = strings(xs).any(|c| c.enc() == Encoding::Utf8);
        }
        // (latin1locale is not modelled: never)
        if !use_utf8 {
            use_utf8 = strings(xs).any(|c| c.enc() == Encoding::Latin1);
        }
    }
    let mut use_wc = false;
    if !fixed && !perl {
        use_wc = use_utf8;
        use_utf8 = false;
    }
    let mode = if use_bytes {
        Mode::Bytes
    } else if use_wc {
        Mode::Wide
    } else if use_utf8 {
        Mode::Utf8
    } else {
        Mode::Native
    };
    (mode, have_bytes)
}

/// A string's units, or the warning its failed translation gives (if
/// any), or an error.
enum Units {
    Ok(Vec<u32>),
    Invalid(Option<String>),
    Error(String),
}

/// iconv's view of `b` in UTF-8: the code points, or the text with each
/// byte it cannot convert written `<xx>`.
fn from_utf8(b: &[u8]) -> Result<Vec<u32>, String> {
    if let Ok(s) = std::str::from_utf8(b) {
        return Ok(s.chars().map(|c| c as u32).collect());
    }
    let mut out = String::new();
    let mut rest = b;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                break;
            }
            Err(e) => {
                let ok = e.valid_up_to();
                out.push_str(std::str::from_utf8(&rest[..ok]).unwrap());
                // iconv stops at the bad byte; R writes it and goes on
                out.push_str(&format!("<{:02x}>", rest[ok]));
                rest = &rest[ok + 1..];
            }
        }
    }
    Err(out)
}

/// CP1252 (what glibc's iconv reads latin1 strings as) to code points, or
/// the text with each unassigned byte written `<xx>`.
// R: sysutils.c:translateToWchar (HAVE_ICONV_CP1252)
fn from_cp1252(b: &[u8]) -> Result<Vec<u32>, String> {
    const HIGH: [u32; 32] = [
        0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D,
        0, 0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0,
        0x017E, 0x0178,
    ];
    let cp = |x: u8| if (0x80..0xA0).contains(&x) { HIGH[(x - 0x80) as usize] } else { x as u32 };
    if b.iter().all(|&x| cp(x) != 0) {
        return Ok(b.iter().map(|&x| cp(x)).collect());
    }
    let mut out = String::new();
    for &x in b {
        match cp(x) {
            0 => out.push_str(&format!("<{x:02x}>")),
            u => out.push(char::from_u32(u).unwrap()),
        }
    }
    Err(out)
}

/// The C locale's iconv from native (ASCII): every high byte fails.
fn from_ascii(b: &[u8]) -> Result<Vec<u32>, String> {
    if b.is_ascii() {
        return Ok(b.iter().map(|&x| x as u32).collect());
    }
    let mut out = String::new();
    for &x in b {
        if x < 0x80 {
            out.push(x as char);
        } else {
            out.push_str(&format!("<{x:02x}>"));
        }
    }
    Err(out)
}

/// The warning translateToWchar/translateToUTF8 give with what they could
/// not convert: at most 255 bytes, else 252 cut to whole characters
/// (`mbcsTruncateToValid`) and "...".
// R: sysutils.c:translateToWchar, util.c:mbcsTruncateToValid
fn translate_warning(text: &str, to: &str) -> String {
    let shown = if text.len() > 255 {
        let mut end = 252;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &text[..end])
    } else {
        text.to_string()
    };
    format!("unable to translate '{shown}' to {to}")
}

/// `c`'s units in `mode`: `CHAR(c)` (bytes), `wtransChar2(c)` (wide),
/// `trCharUTF82(c)` checked by `utf8Valid` (UTF-8, as code points), or
/// `translateCharFP2(c)` checked by `mbcsValid` (native). `c` is not NA.
// R: sysutils.c:wtransChar2, trCharUTF82, translateCharFP2
fn units(c: Char<'_>, mode: Mode, utf8_locale: bool) -> Units {
    let b = c.bytes();
    let bytes = || b.iter().map(|&x| x as u32).collect::<Vec<u32>>();
    if mode == Mode::Bytes || c.is_ascii() {
        return Units::Ok(bytes());
    }
    if c.enc() == Encoding::Bytes {
        // (input marked "bytes" means useBytes)
        return Units::Error("translating strings with \"bytes\" encoding is not allowed".into());
    }
    let latin1 = c.enc() == Encoding::Latin1;
    let tr = match mode {
        // wneedsTranslation: latin1, UTF-8 (marked, or the locale's), or
        // the C locale's native
        Mode::Wide => match c.enc() {
            Encoding::Latin1 => from_cp1252(b),
            Encoding::Native if !utf8_locale => from_ascii(b),
            _ => from_utf8(b),
        }
        .map_err(|t| Some(translate_warning(&t, "a wide string"))),
        // needsTranslationUTF8: none for UTF-8 or the UTF-8 locale's
        // native (then utf8Valid decides, silently)
        Mode::Utf8 => match c.enc() {
            Encoding::Latin1 => from_cp1252(b).map_err(|t| Some(translate_warning(&t, "UTF-8"))),
            Encoding::Native if !utf8_locale => from_ascii(b).map_err(|t| Some(translate_warning(&t, "UTF-8"))),
            _ => from_utf8(b).map_err(|_| None),
        },
        // only native strings get here (a marked one makes use_UTF8):
        // mbcsValid in the UTF-8 locale, bytes in the C locale
        Mode::Native | Mode::Bytes => {
            if utf8_locale && std::str::from_utf8(b).is_err() {
                Err(None)
            } else {
                Ok(bytes())
            }
        }
    };
    match tr {
        Ok(u) => Units::Ok(u),
        // (the message would be re-encoded to ASCII with substitutions)
        Err(Some(_)) if latin1 && !utf8_locale => Units::Error(
            "robigo: a regular expression on a latin1 string iconv cannot convert, in the C locale, is not \
             implemented yet (INTERIM(RX))"
                .into(),
        ),
        Err(w) => Units::Invalid(w),
    }
}

/// [`units`] given: its translation warning, its error; None when invalid.
fn units_or_warn(cx: &mut Cx, u: Units) -> RResult<Option<Vec<u32>>> {
    match u {
        Units::Ok(u) => Ok(Some(u)),
        Units::Invalid(w) => {
            if let Some(w) = w {
                cx.warning(&w)?;
            }
            Ok(None)
        }
        Units::Error(e) => Err(cx.error(&e)),
    }
}

/// Units back to a string's bytes: bytes as they are, code points in
/// UTF-8.
fn to_bytes(u: &[u32], mode: Mode) -> Vec<u8> {
    if mode.wide() {
        u.iter().filter_map(|&x| char::from_u32(x)).collect::<String>().into_bytes()
    } else {
        u.iter().map(|&x| x as u8).collect()
    }
}

/// `fgrep_one`: the first position of `pat` in `s` at or after `from`.
/// (Over bytes of valid UTF-8, or code points, a byte search finds the
/// same matches as GNU R's character steps.)
// R: grep.c:fgrep_one, fgrep_one_bytes
fn fgrep_one(pat: &[u32], s: &[u32], from: usize) -> Option<usize> {
    if pat.is_empty() {
        return Some(from);
    }
    if s.len() < pat.len() {
        return None;
    }
    (from..=s.len() - pat.len()).find(|&i| s[i..i + pat.len()] == *pat)
}

/// grep.c's `_R_REGEX_MARK_{NEW,OLD}_RESULT_AS_BYTES_` switches
/// (`mkBytesNew`, `markBytesOld`), read once.
fn mark_as_bytes(new: bool) -> bool {
    static NEW: OnceLock<bool> = OnceLock::new();
    static OLD: OnceLock<bool> = OnceLock::new();
    let (cell, var) =
        if new { (&NEW, "_R_REGEX_MARK_NEW_RESULT_AS_BYTES_") } else { (&OLD, "_R_REGEX_MARK_OLD_RESULT_AS_BYTES_") };
    // StringTrue
    *cell.get_or_init(|| {
        std::env::var_os(var).is_some_and(|v| matches!(v.as_encoded_bytes(), b"T" | b"True" | b"TRUE" | b"true"))
    })
}

/// The checks `do_grep` and `do_gsub` make of `pattern`.
fn check_pattern(cx: &mut Cx, pat: Root<'_>) -> RResult<()> {
    let p = pat.bind(cx);
    let n = p.len().unwrap_or(0);
    if p.sexptype() != STRSXP || n < 1 {
        return Err(cx.error("invalid 'pattern' argument"));
    }
    if n > 1 {
        cx.warning("argument 'pattern' has length > 1 and only the first element will be used")?;
    }
    Ok(())
}

/// The pattern compiled in TRE's place, after the interim's PCRE and
/// `ignore.case` refusals.
fn compile(cx: &mut Cx, p: &[u32], perl: bool, igcase: bool, what: &str) -> RResult<ere::Re> {
    if perl {
        return Err(not_rx(cx, &format!("{what}(perl = TRUE)")));
    }
    if igcase {
        return Err(not_rx(cx, &format!("{what}(ignore.case = TRUE)")));
    }
    ere::compile(p).map_err(|w| not_rx(cx, w))
}

/// `x`'s names (`getAttrib(x, R_NamesSymbol)`) installed on `a`: subset to
/// `keep`, or all of them (`duplicate(nmold)`).
fn names_into<'g>(
    cx: &'g Cx,
    a: &mut Attrs<'g>,
    x: Ref<'g>,
    keep: Option<&[usize]>,
) -> Result<(), robigo_rtcore::NeedGc> {
    let Some(nm) = attrib::names(x) else { return Ok(()) };
    let v = nm.as_vector::<Char<'g>>().unwrap();
    let all: Vec<usize>;
    let keep = match keep {
        Some(k) => k,
        None => {
            all = (0..v.len()).collect();
            &all
        }
    };
    let mut s = cx.alloc_vector::<Char<'g>>(keep.len())?;
    for (j, &i) in keep.iter().enumerate() {
        s.set(j, v.get(i));
    }
    a.install(Attr { tag: cx.sym(RSym::Names), value: s.finish().into() });
    Ok(())
}

/// `.Internal(grep(pattern, x, ignore.case, value, perl, fixed, useBytes,
/// invert))` (`CODE` 0) and `grepl` (1: a logical for each element).
// R: grep.c:do_grep
pub fn do_grep<const CODE: u8>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let grepl = CODE == 1;
    let row = if grepl { crate::row!("grepl", true) } else { crate::row!("grep", true) };
    row.check_arity(cx, call, args)?;
    let (pat, text) = (args.root(0), args.root(1));
    let igcase = as_bool2(cx, call, args.root(2))?;
    let value = as_bool2(cx, call, args.root(3))?;
    let mut perl = as_bool2(cx, call, args.root(4))?;
    let fixed = as_bool2(cx, call, args.root(5))?;
    let use_bytes = as_bool2(cx, call, args.root(6))?;
    let invert = as_bool2(cx, call, args.root(7))?;
    if fixed && igcase {
        cx.warning("argument 'ignore.case = TRUE' will be ignored")?;
    }
    if fixed && perl {
        cx.warning("argument 'perl = TRUE' will be ignored")?;
        perl = false;
    }
    check_pattern(cx, pat)?;
    if text.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("invalid 'text' argument"));
    }
    let n = text.bind(cx).len().unwrap_or(0);
    let p0 = pat.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
    if p0 == cx.na_string() {
        return cx.build(out, |cx| {
            if value {
                let mut v = cx.alloc_vector::<Char<'_>>(n)?;
                for i in 0..n {
                    v.set(i, cx.na_string());
                }
                let mut a = Attrs::new();
                names_into(cx, &mut a, text.bind(cx), None)?;
                a.apply(cx, &mut v)?;
                Ok(v.finish().into())
            } else if grepl {
                Ok(cx.alloc_with(n, |_| Lgl(NA_INTEGER))?.finish().into())
            } else {
                Ok(cx.alloc_with(n, |_| NA_INTEGER)?.finish().into())
            }
        });
    }
    let (mode, _) = decide(cx, &[(pat.bind(cx), 1), (text.bind(cx), n)], use_bytes, fixed, perl);
    let loc = cx.locale().utf8;
    let Some(spat) = units_or_warn(cx, units(p0, mode, loc))? else {
        let m = match mode {
            Mode::Utf8 => "regular expression is invalid UTF-8",
            Mode::Native => "regular expression is invalid in this locale",
            _ => "regular expression is invalid",
        };
        return Err(cx.error(m));
    };
    let re = if fixed { None } else { Some(compile(cx, &spat, perl, igcase, "grep")?) };
    let mut ind = vec![false; n];
    let mut nwarn = 0;
    for (i, hit) in ind.iter_mut().enumerate() {
        let c = text.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        if c == cx.na_string() {
            continue;
        }
        let Some(s) = units_or_warn(cx, units(c, mode, loc))? else {
            nwarn += 1;
            if nwarn <= NWARN {
                let m = match mode {
                    Mode::Utf8 => "is invalid UTF-8",
                    Mode::Native => "is invalid in this locale",
                    _ => "is invalid",
                };
                cx.warning(&format!("input string {} {m}", i + 1))?;
            }
            continue;
        };
        *hit = match &re {
            None => fgrep_one(&spat, &s, 0).is_some(),
            Some(re) => re.find(&s, 0, false).is_some(),
        };
    }
    if grepl {
        return cx.build(out, |cx| Ok(cx.alloc_with(n, |i| Lgl(ind[i] as i32))?.finish().into()));
    }
    let keep: Vec<usize> = (0..n).filter(|&i| invert ^ ind[i]).collect();
    cx.build(out, |cx| {
        if value {
            let tv = text.bind(cx);
            let t = tv.as_vector::<Char<'_>>().unwrap();
            let mut v = cx.alloc_vector::<Char<'_>>(keep.len())?;
            for (j, &i) in keep.iter().enumerate() {
                v.set(j, t.get(i));
            }
            // copy across names and subset
            let mut a = Attrs::new();
            names_into(cx, &mut a, tv, Some(&keep))?;
            a.apply(cx, &mut v)?;
            Ok(v.finish().into())
        } else {
            Ok(cx.alloc_with(keep.len(), |j| (keep[j] + 1) as i32)?.finish().into())
        }
    })
}

/// `string_adj`/`wstring_adj`: the replacement `rep` for a match; `\1` to
/// `\9` the subexpressions (none here: empty), `\` before another
/// character that character, a final `\` dropped.
// R: grep.c:string_adj, wstring_adj
fn adj(out: &mut Vec<u32>, rep: &[u32]) {
    let bs = '\\' as u32;
    let mut k = 0;
    while k < rep.len() {
        if rep[k] != bs {
            out.push(rep[k]);
            k += 1;
            continue;
        }
        match rep.get(k + 1) {
            Some(&d) if ('1' as u32..='9' as u32).contains(&d) => k += 2,
            None => k += 1,
            Some(&d) => {
                out.push(d);
                k += 2;
            }
        }
    }
}

/// One element's result in `do_gsub`.
enum Res {
    /// No match: the input (`markBytesOld`).
    Same,
    Na,
    /// The new string's bytes and encoding.
    New(Vec<u8>, Encoding),
}

/// `.Internal(sub(pattern, replacement, x, ignore.case, perl, fixed,
/// useBytes))` (`CODE` 0) and `gsub` (1: every match).
// R: grep.c:do_gsub
pub fn do_gsub<const CODE: u8>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let global = CODE == 1;
    let row = if global { crate::row!("gsub", true) } else { crate::row!("sub", true) };
    row.check_arity(cx, call, args)?;
    let (pat, rep, text) = (args.root(0), args.root(1), args.root(2));
    let igcase = as_bool2(cx, call, args.root(3))?;
    let mut perl = as_bool2(cx, call, args.root(4))?;
    let fixed = as_bool2(cx, call, args.root(5))?;
    let use_bytes = as_bool2(cx, call, args.root(6))?;
    if fixed && igcase {
        cx.warning("argument 'ignore.case = TRUE' will be ignored")?;
    }
    if fixed && perl {
        cx.warning("argument 'perl = TRUE' will be ignored")?;
        perl = false;
    }
    check_pattern(cx, pat)?;
    let r = rep.bind(cx);
    let nr = r.len().unwrap_or(0);
    if r.sexptype() != STRSXP || nr < 1 {
        return Err(cx.error("invalid 'replacement' argument"));
    }
    if nr > 1 {
        cx.warning("argument 'replacement' has length > 1 and only the first element will be used")?;
    }
    if text.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("invalid 'text' argument"));
    }
    let n = text.bind(cx).len().unwrap_or(0);
    let p0 = pat.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
    let rep_na = rep.bind(cx).as_vector::<Char<'_>>().unwrap().get(0) == cx.na_string();
    let mut res: Vec<Res> = Vec::with_capacity(n);
    // (an NA pattern: every result NA)
    if p0 != cx.na_string() {
        let xs = [(pat.bind(cx), 1), (rep.bind(cx), 1), (text.bind(cx), n)];
        let (mode, have_bytes) = decide(cx, &xs, use_bytes, fixed, perl);
        let invalid = |what: &str| match mode {
            Mode::Utf8 => format!("'{what}' is invalid UTF-8"),
            Mode::Native => format!("'{what}' is invalid in this locale"),
            _ => format!("'{what}' is invalid"),
        };
        let loc = cx.locale().utf8;
        let Some(spat) = units_or_warn(cx, units(p0, mode, loc))? else {
            return Err(cx.error(&invalid("pattern")));
        };
        // (an NA replacement is its "NA")
        let r0 = rep.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
        let Some(srep) = units_or_warn(cx, units(r0, mode, loc))? else {
            return Err(cx.error(&invalid("replacement")));
        };
        let re = if fixed {
            if spat.is_empty() {
                return Err(cx.error("zero-length pattern"));
            }
            None
        } else {
            Some(compile(cx, &spat, perl, igcase, if global { "gsub" } else { "sub" })?)
        };
        // the new strings: mkBytesNew, mkCharWLenASCII (UTF-8 unless
        // ASCII), mkCharCE(CE_UTF8), markKnown (of a native input: native)
        let enc = match mode {
            Mode::Bytes if have_bytes || mark_as_bytes(true) => Encoding::Bytes,
            Mode::Bytes | Mode::Native => Encoding::Native,
            Mode::Wide | Mode::Utf8 => Encoding::Utf8,
        };
        for i in 0..n {
            let c = text.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
            if c == cx.na_string() {
                res.push(Res::Na);
                continue;
            }
            let Some(s) = units_or_warn(cx, units(c, mode, loc))? else {
                let m = match mode {
                    Mode::Utf8 => "is invalid UTF-8",
                    Mode::Native => "is invalid in this locale",
                    _ => "is invalid",
                };
                return Err(cx.error(&format!("input string {} {m}", i + 1)));
            };
            let mut u: Vec<u32> = Vec::with_capacity(s.len());
            let mut nmatch = 0;
            let mut offset = 0;
            match &re {
                None => {
                    while let Some(st) = fgrep_one(&spat, &s, offset) {
                        nmatch += 1;
                        u.extend_from_slice(&s[offset..st]);
                        u.extend_from_slice(&srep);
                        offset = st + spat.len();
                        if !global {
                            break;
                        }
                    }
                }
                Some(re) => {
                    let (mut notbol, mut last_end) = (false, None);
                    while let Some((so, eo)) = re.find(&s, offset, notbol) {
                        nmatch += 1;
                        u.extend_from_slice(&s[offset..so]);
                        if last_end.is_none_or(|l| eo > l) {
                            adj(&mut u, &srep);
                            last_end = Some(eo);
                        }
                        offset = eo;
                        if offset >= s.len() || !global {
                            break;
                        }
                        if eo == so {
                            u.push(s[offset]);
                            offset += 1;
                        }
                        notbol = true;
                    }
                }
            }
            if nmatch == 0 {
                res.push(Res::Same);
            } else if rep_na {
                res.push(Res::Na);
            } else {
                u.extend_from_slice(&s[offset..]);
                res.push(Res::New(to_bytes(&u, mode), enc));
            }
        }
        // markBytesOld: a no-match input re-marked "bytes" only by its switch
        if mode == Mode::Bytes && have_bytes && mark_as_bytes(false) {
            let t = text.bind(cx).as_vector::<Char<'_>>().unwrap();
            let na = cx.na_string();
            for (i, r) in res.iter_mut().enumerate() {
                let c = t.get(i);
                if matches!(r, Res::Same) && !c.is_ascii() && c.enc() != Encoding::Bytes && c != na {
                    *r = Res::New(c.bytes().to_vec(), Encoding::Bytes);
                }
            }
        }
    }
    cx.build(out, |cx| {
        let tv = text.bind(cx);
        let t = tv.as_vector::<Char<'_>>().unwrap();
        let mut v = cx.alloc_vector::<Char<'_>>(n)?;
        for i in 0..n {
            let c = match res.get(i) {
                None | Some(Res::Na) => cx.na_string(),
                Some(Res::Same) => t.get(i),
                Some(Res::New(b, e)) => cx.mk_char(b, *e)?,
            };
            v.set(i, c);
        }
        // SHALLOW_DUPLICATE_ATTRIB(ans, text)
        let mut a = Attrs::new();
        a.copy_all(tv);
        a.apply(cx, &mut v)?;
        Ok(v.finish().into())
    })
}
