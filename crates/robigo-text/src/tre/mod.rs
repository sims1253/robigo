//! TRE, the POSIX regex engine GNU R uses for `perl = FALSE`, ported from
//! R 4.6.2's patched copy (`src/extra/tre`).
//!
//! The pipeline is TRE's: [`parse`] builds a syntax tree, `compile` tags,
//! expands and converts it to a tagged NFA, and the matchers run it.

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
    Assert {
        expr: &'static str,
        file: &'static str,
        line: u32,
    },
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

pub use exec::{Match, Text};

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
        Ok(Regex {
            tnfa: c.tnfa,
            re_nsub: c.re_nsub,
            locale,
        })
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

    /// Matches a wide string up to its end.
    // R: regexec.c:tre_regwexec
    pub fn exec_wide(&self, s: &[i32], pmatch: &mut [Match], eflags: i32) -> Result<bool, Error> {
        exec::tre_match(&self.tnfa, self.locale, s, -1, pmatch, eflags)
    }
}
