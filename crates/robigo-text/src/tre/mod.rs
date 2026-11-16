//! TRE, the POSIX regex engine GNU R uses for `perl = FALSE`, ported from
//! R 4.6.2's patched copy (`src/extra/tre`).
//!
//! The pipeline is TRE's: [`parse`] builds a syntax tree, `compile` tags,
//! expands and converts it to a tagged NFA, and the matchers run it.

mod ast;
pub(crate) mod ctype;
mod parse;
mod tables;

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
