//! TRE, the POSIX regex engine GNU R uses for `perl = FALSE`, ported from
//! R 4.6.2's patched copy (`src/extra/tre`).
//!
//! The pipeline is TRE's: [`parse`] builds a syntax tree, `compile` tags,
//! expands and converts it to a tagged NFA, and the matchers run it.

mod approx;
mod ast;
mod compile;
pub(crate) mod ctype;
mod exec;
mod parse;
mod tables;

use crate::Locale;

/// Why compiling or matching failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A `reg_errcode_t`; R reports `message()` as the reason.
    Status(Status),
    /// A failed `R_assert`, which R turns into an error.
    Assert { expr: &'static str, file: &'static str, line: u32 },
    /// A TRE feature robigo has not ported.
    Unsupported(&'static str),
}

impl From<Status> for Error {
    fn from(s: Status) -> Error {
        Error::Status(s)
    }
}

impl Error {
    /// The text R's error reports.
    // R: main/errors.c:R_assert (via TRE's `#define assert`)
    pub fn message(&self) -> String {
        match *self {
            Error::Status(s) => s.message().to_string(),
            Error::Assert { expr, file, line } => {
                format!("assertion '{expr}' failed in executing regexp: file '{file}', line {line}\n")
            }
            Error::Unsupported(what) => format!("robigo: '{what}' is not implemented yet"),
        }
    }
}

/// `reg_errcode_t` (tre.h): the status codes `tre_regcomp` and
/// `tre_regexec` return. `REG_OK` is `Ok(..)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Nomatch = 1,
    Badpat = 2,
    Ecollate = 3,
    Ectype = 4,
    Eescape = 5,
    Esubreg = 6,
    Ebrack = 7,
    Eparen = 8,
    Ebrace = 9,
    Badbr = 10,
    Erange = 11,
    Espace = 12,
    Badrpt = 13,
}

impl Status {
    /// `tre_regerror`'s text, untranslated (R builds TRE without NLS
    /// catalogs of its own).
    // R: regerror.c:tre_error_messages
    pub fn message(self) -> &'static str {
        match self {
            Status::Nomatch => "No match",
            Status::Badpat => "Invalid regexp",
            Status::Ecollate => "Unknown collating element",
            Status::Ectype => "Unknown character class name",
            Status::Eescape => "Trailing backslash",
            Status::Esubreg => "Invalid back reference",
            Status::Ebrack => "Missing ']'",
            Status::Eparen => "Missing ')'",
            Status::Ebrace => "Missing '}'",
            Status::Badbr => "Invalid contents of {}",
            Status::Erange => "Invalid character range",
            Status::Espace => "Out of memory",
            Status::Badrpt => "Invalid use of repetition operators",
        }
    }
}

/// Compilation flags (`cflags`, tre.h).
pub const REG_EXTENDED: i32 = 1;
pub const REG_ICASE: i32 = 2;
pub const REG_NEWLINE: i32 = 4;
pub const REG_NOSUB: i32 = 8;
pub const REG_LITERAL: i32 = 16;
pub const REG_RIGHT_ASSOC: i32 = 32;
pub const REG_UNGREEDY: i32 = 64;
pub const REG_USEBYTES: i32 = 128;

/// Execution flags (`eflags`, tre.h).
pub const REG_NOTBOL: i32 = 1;
pub const REG_NOTEOL: i32 = 2;
pub const REG_APPROX_MATCHER: i32 = 4;
pub const REG_BACKTRACKING_MATCHER: i32 = 8;

pub use approx::{AMatch, AParams};
/// glibc's `wctype` classes, `iswspace`, `towupper` and `towlower` in a locale.
pub use ctype::{Class, iswspace, tolower as towlower, toupper as towupper};
pub use exec::Match;
use exec::Mbs;

/// A compiled regex (`regex_t`).
pub struct Regex {
    tnfa: compile::Tnfa,
    re_nsub: usize,
    locale: Locale,
}

impl Regex {
    /// Compiles a pattern of wide characters: `tre_regwcomp`, or
    /// `tre_regcomp` in a single-byte locale with the bytes widened.
    // R: regcomp.c:tre_regwncomp
    pub fn new(pat: &[i32], cflags: i32, locale: Locale) -> Result<Regex, Error> {
        let c = compile::compile(pat, cflags, locale)?;
        Ok(Regex { tnfa: c.tnfa, re_nsub: c.re_nsub, locale })
    }

    /// Compiles a pattern taking its bytes literally.
    // R: regcomp.c:tre_regncompb
    pub fn new_bytes(pat: &[u8], cflags: i32, locale: Locale) -> Result<Regex, Error> {
        let wide: Vec<i32> = pat.iter().map(|&b| b as i32).collect();
        Regex::new(&wide, cflags | REG_USEBYTES, locale)
    }

    /// The number of parenthesized subexpressions.
    pub fn re_nsub(&self) -> usize {
        self.re_nsub
    }

    // R: regexec.c:tre_have_backrefs
    pub fn have_backrefs(&self) -> bool {
        self.tnfa.have_backrefs
    }

    /// True when GNU R's TRE reads uninitialized memory matching this
    /// regex, so its results vary from run to run: nested approximate
    /// parameters where an empty path sets only some of them (C's
    /// `tre_set_union` does not copy the rest). robigo keeps the outer
    /// values for those.
    pub fn undefined_in_c(&self) -> bool {
        self.tnfa.uninit_params
    }

    /// Matches a byte string up to its end: `tre_regexecb`, or
    /// `tre_regexec` in a single-byte locale. `Ok(false)` is no match.
    // R: regexec.c:tre_regexecb
    pub fn exec_bytes(&self, s: &[u8], pmatch: &mut [Match], eflags: i32) -> Result<bool, Error> {
        exec::tre_match(&self.tnfa, self.locale, s, -1, pmatch, eflags)
    }

    /// Matches exactly the bytes of `s`, which may contain NULs.
    // R: regexec.c:tre_regnexecb
    pub fn exec_bytes_n(&self, s: &[u8], pmatch: &mut [Match], eflags: i32) -> Result<bool, Error> {
        let len = i32::try_from(s.len()).map_err(|_| Error::Status(Status::Espace))?;
        exec::tre_match(&self.tnfa, self.locale, s, len, pmatch, eflags)
    }

    /// Matches a string in the locale's multibyte encoding up to its end:
    /// `tre_regexec` in a UTF-8 locale. Positions are byte offsets.
    // R: regexec.c:tre_regexec (STR_MBS)
    pub fn exec_mbs(&self, s: &[u8], pmatch: &mut [Match], eflags: i32) -> Result<bool, Error> {
        exec::tre_match(&self.tnfa, self.locale, &Mbs(s), -1, pmatch, eflags)
    }

    /// Matches a wide string up to its end.
    // R: regexec.c:tre_regwexec
    pub fn exec_wide(&self, s: &[i32], pmatch: &mut [Match], eflags: i32) -> Result<bool, Error> {
        exec::tre_match(&self.tnfa, self.locale, s, -1, pmatch, eflags)
    }

    /// Matches a byte string approximately: `tre_regaexecb`, or
    /// `tre_regaexec` in a single-byte locale. `Ok(None)` is no match.
    // R: regexec.c:tre_regaexecb
    pub fn aexec_bytes(
        &self,
        s: &[u8],
        pmatch: &mut [Match],
        params: &AParams,
        eflags: i32,
    ) -> Result<Option<AMatch>, Error> {
        exec::tre_match_approx(&self.tnfa, self.locale, s, -1, pmatch, params, eflags)
    }

    /// Matches a string in the locale's multibyte encoding approximately:
    /// `tre_regaexec` in a UTF-8 locale.
    // R: regexec.c:tre_regaexec (STR_MBS)
    pub fn aexec_mbs(
        &self,
        s: &[u8],
        pmatch: &mut [Match],
        params: &AParams,
        eflags: i32,
    ) -> Result<Option<AMatch>, Error> {
        exec::tre_match_approx(&self.tnfa, self.locale, &Mbs(s), -1, pmatch, params, eflags)
    }

    /// Matches a wide string approximately.
    // R: regexec.c:tre_regawexec
    pub fn aexec_wide(
        &self,
        s: &[i32],
        pmatch: &mut [Match],
        params: &AParams,
        eflags: i32,
    ) -> Result<Option<AMatch>, Error> {
        exec::tre_match_approx(&self.tnfa, self.locale, s, -1, pmatch, params, eflags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pat: &str, s: &str, cflags: i32) -> Option<Vec<(i32, i32)>> {
        let w: Vec<i32> = pat.chars().map(|c| c as i32).collect();
        let re = Regex::new(&w, cflags, Locale::Utf8).unwrap();
        let mut pm = vec![Match::UNSET; re.re_nsub() + 1];
        let ws: Vec<i32> = s.chars().map(|c| c as i32).collect();
        re.exec_wide(&ws, &mut pm, 0).unwrap().then(|| pm.iter().map(|m| (m.so, m.eo)).collect())
    }

    #[test]
    fn smoke() {
        assert_eq!(run("a(b*)c", "xabbc", REG_EXTENDED), Some(vec![(1, 5), (2, 4)]));
        assert_eq!(run("(a|ab)(c|bcd)", "abcd", REG_EXTENDED), Some(vec![(0, 4), (0, 1), (1, 4)]));
        assert_eq!(run("(a+)b\\1", "xaabaa", REG_EXTENDED), Some(vec![(1, 6), (1, 3)]));
        assert_eq!(run("^x", "ax", REG_EXTENDED), None);
        assert_eq!(run("A", "xa", REG_EXTENDED | REG_ICASE), Some(vec![(1, 2)]));
        assert_eq!(run("abc{~1}", "abd", REG_EXTENDED), Some(vec![(0, 3)]));
        assert_eq!(run("colour{~1}", "color", REG_EXTENDED), None);
        assert_eq!(run("(colour){~1}", "color", REG_EXTENDED), Some(vec![(0, 5), (0, 5)]));
        assert_eq!(run("abc{~1}", "xyz", REG_EXTENDED), None);
    }
}
