//! Syntax trees shaped like R's own language objects.
//!
//! GNU R represents parsed code as calls, symbols and constants; `if`, `{`,
//! `<-` and friends are just calls to functions with those names. We keep the
//! same shape so the runtime can turn an [`Expr`] into a language object
//! without any lowering decisions of its own. The only node that is not a
//! plain call is [`Expr::Function`], because R stores formals as a pairlist,
//! which the runtime builds itself.

use std::fmt;

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
#[derive(Clone, Debug, PartialEq)]
pub enum Const {
    Null,
    Lgl(Option<bool>),
    Int(Option<i32>),
    Dbl(f64),
    /// `NA_real_` is distinct from `NaN`; R encodes it as a NaN payload.
    DblNa,
    Cplx(f64, f64),
    CplxNa,
    Str(Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub name: Option<String>,
    /// `None` is the empty argument, as in `x[, 1]`.
    pub value: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Formal {
    pub name: String,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Const(Const, Span),
    Sym(String, Span),
    Call { fun: Box<Expr>, args: Vec<Arg>, span: Span },
    Function { formals: Vec<Formal>, body: Box<Expr>, span: Span },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Const(_, s) | Expr::Sym(_, s) => *s,
            Expr::Call { span, .. } | Expr::Function { span, .. } => *span,
        }
    }

    pub fn call(name: &str, name_span: Span, args: Vec<Arg>, span: Span) -> Expr {
        Expr::Call { fun: Box::new(Expr::Sym(name.to_string(), name_span)), args, span }
    }

    /// The function name, if this is a call to a symbol.
    pub fn call_name(&self) -> Option<&str> {
        match self {
            Expr::Call { fun, .. } => match fun.as_ref() {
                Expr::Sym(s, _) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }
}

impl Arg {
    pub fn positional(value: Expr) -> Arg {
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
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}
