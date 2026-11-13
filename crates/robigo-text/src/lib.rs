//! GNU R's number and string formatting, ported from R 4.6.2's C
//! (`src/main/format.c`, `src/main/printutils.c`) over plain doubles,
//! integers and byte strings. No R objects: the caller passes what the C
//! reads from a SEXP (a CHARSXP's bytes and encoding mark) or from global
//! state (`R_print`, `OutDec`, the locale) as values.
//!
//! - [`num`]: `formatReal`/`scientific`, `formatInteger`, `formatLogical`
//!   decide a common width; `encodeReal0`, `EncodeRealDrop0`,
//!   `EncodeInteger` and `EncodeLogical` encode one element in it.
//! - [`string`]: `Rstrwid`/`Rstrlen` (the escaped width of a string),
//!   `EncodeString` (escaping, quoting, padding) and `formatString`.
//! - [`Str`], [`Enc`] and [`Locale`]: a CHARSXP's bytes and mark, and the
//!   C library's view of the locale (`mbrtowc`, `iswprint`, `isprint`,
//!   R's `Ri18n_wcwidth`, `translateChar`).
//!
//! Encoders append to a caller's `Vec<u8>`, so a printing loop reuses one
//! buffer. The expected values in the tests are recorded from GNU R 4.6.2
//! by `oracle/record.sh`.

#![forbid(unsafe_code)]

// The checkout this crate is built from (.cargo/config.toml): cargo then
// rebuilds the crate when another worktree built it last in a shared
// target directory.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");

mod cfmt;
mod locale;
pub mod num;
pub mod string;
mod tables;

pub use locale::{Enc, Locale, Str, TranslateError, translate_char};

/// The parts of R's `R_print` (Print.h) and of the C library's locale that
/// the formatters read. [`PrintParams::DEFAULT`] is `PrintDefaults()` with
/// `options()` at their defaults, in a UTF-8 locale.
#[derive(Clone, Copy, Debug)]
pub struct PrintParams<'a> {
    /// `R_print.digits`: significant digits, 1 to 22 (R validates
    /// `digits` arguments to that range before formatting).
    pub digits: i32,
    /// `R_print.scipen`: the penalty for scientific notation.
    pub scipen: i32,
    /// `R_print.na_string`: how numbers, and quoted strings, print `NA`.
    pub na_string: Str<'a>,
    /// `R_print.na_string_noquote`: how unquoted strings print `NA`.
    pub na_string_noquote: Str<'a>,
    /// `R_print.na_width`: the width `na_string` pads to.
    pub na_width: i32,
    /// `R_print.na_width_noquote`.
    pub na_width_noquote: i32,
    /// The C library's `LC_CTYPE`.
    pub locale: Locale,
}

impl PrintParams<'static> {
    /// `PrintDefaults()`: 7 digits, `scipen` 0, `NA` and `<NA>`.
    // R: print.c:PrintDefaults
    pub const DEFAULT: PrintParams<'static> = PrintParams {
        digits: 7,
        scipen: 0,
        na_string: Str::ascii(b"NA"),
        na_string_noquote: Str::ascii(b"<NA>"),
        na_width: 2,
        na_width_noquote: 4,
        locale: Locale::Utf8,
    };
}

impl Default for PrintParams<'static> {
    fn default() -> Self {
        PrintParams::DEFAULT
    }
}

pub use robigo_numerics::NA_REAL;

/// R's `NA_integer_` and `NA_logical`: `INT_MIN`.
pub const NA_INTEGER: i32 = i32::MIN;

/// `ISNA(x)`: R's `NA_real_` (as opposed to any other NaN).
// R: arithmetic.c:R_IsNA
#[inline]
pub fn is_na(x: f64) -> bool {
    x.is_nan() && (x.to_bits() as u32) == 1954
}
