//! Syntax trees shaped like R's own language objects.
//!
//! GNU R represents parsed code as calls, symbols and constants; `if`, `{`,
//! `<-` and friends are just calls to functions with those names. We keep the
//! same shape so the runtime can turn an [`Expr`] into a language object
//! without any lowering decisions of its own. The only node that is not a
//! plain call is [`Expr::Function`], because R stores formals as a pairlist,
//! which the runtime builds itself.
//!
//! Nodes live in an [`crate::Arena`] (`'a`), and plain identifiers and
//! strings reference the source text directly; dropping the arena frees the
//! whole tree at once, so no node owns heap memory.

/// Half-open byte range into the parsed source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start: start as u32, end: end as u32 }
    }

    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

/// Literal constants. `NA` variants use `None`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Const<'a> {
    Null,
    Lgl(Option<bool>),
    Int(Option<i32>),
    Dbl(f64),
    /// `NA_real_` is distinct from `NaN`; R encodes it as a NaN payload.
    DblNa,
    Cplx(f64, f64),
    CplxNa,
    /// String content as raw bytes: source text keeps its UTF-8 bytes, and
    /// `\xNN`/`\NNN` escapes produce their single byte (a "bytes"-encoded
    /// CHARSXP in GNU R, whatever the locale).
    Str(Option<&'a [u8]>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arg<'a> {
    pub name: Option<&'a str>,
    /// `None` is the empty argument, as in `x[, 1]`.
    pub value: Option<Expr<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Formal<'a> {
    pub name: &'a str,
    pub default: Option<Expr<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expr<'a> {
    Const(Const<'a>, Span),
    Sym(&'a str, Span),
    /// The pipe placeholder `_`, the string constant `"_"` in GNU R's
    /// trees (`R_PlaceholderToken`). Kept distinct so that only real
    /// placeholders trigger GNU R's post-statement check, and so pipes can
    /// substitute them.
    Placeholder(Span),
    Call {
        fun: &'a Expr<'a>,
        args: &'a [Arg<'a>],
        span: Span,
    },
    Function {
        formals: &'a [Formal<'a>],
        body: &'a Expr<'a>,
        span: Span,
    },
}

impl<'a> Expr<'a> {
    pub fn span(&self) -> Span {
        match self {
            Expr::Const(_, s) | Expr::Sym(_, s) | Expr::Placeholder(s) => *s,
            Expr::Call { span, .. } | Expr::Function { span, .. } => *span,
        }
    }

    /// The function name, if this is a call to a symbol.
    pub fn call_name(&self) -> Option<&str> {
        match self {
            Expr::Call { fun, .. } => match fun {
                Expr::Sym(s, _) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }
}

impl<'a> Arg<'a> {
    pub fn positional(value: Expr<'a>) -> Arg<'a> {
        Arg { name: None, value: Some(value) }
    }
}

/// A parse error with a byte offset into the source.
#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub offset: usize,
    /// Byte offset just past the offending token.
    pub end: usize,
    /// True when the input ended before the expression was complete; a REPL
    /// should read another line instead of reporting the error.
    pub incomplete: bool,
    /// True when the position is the end-of-input token: GNU R's lexer
    /// appends a virtual newline when the source does not end with one, so
    /// the error sits on the following line at column 0.
    pub at_eof: bool,
    /// True for errors raised from parser actions (`raiseParseError` in
    /// gram.y), whose message carries a `" (<input>:<line>:<col>)"` suffix.
    pub with_pos_suffix: bool,
    /// A position not derived from `offset`, as when GNU R reports the
    /// statement terminator for pipe-placeholder misuse.
    pub fixed_line_col: Option<(usize, usize)>,
}

impl ParseError {
    pub fn new(message: impl Into<String>, offset: usize, end: usize) -> Self {
        ParseError {
            message: message.into(),
            offset,
            end,
            incomplete: false,
            at_eof: false,
            with_pos_suffix: false,
            fixed_line_col: None,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

use std::fmt;
