//! Conversion from syntax trees to R language objects.

use robigo_syntax::{Const, Expr};

use crate::sym::{Sym, s};
use crate::value::*;

pub fn const_to_value(c: &Const) -> Value {
    match c {
        Const::Null => Value::Null,
        Const::Lgl(None) => Value::lgl_na(),
        Const::Lgl(Some(b)) => Value::lgl(*b),
        Const::Int(None) => Value::int(NA_INTEGER),
        Const::Int(Some(i)) => Value::int(*i),
        Const::Dbl(x) => Value::real(*x),
        Const::DblNa => Value::real(NA_REAL),
        Const::Cplx(re, im) => Value::cplx(Complex::new(*re, *im)),
        Const::CplxNa => Value::cplx(Complex::NA),
        Const::Str(None) => Value::str_na(),
        // GNU R keeps escaped bytes as a "bytes"-encoded CHARSXP; until the
        // value model carries encodings, such bytes become their Latin-1
        // characters so no information is lost in the common case.
        Const::Str(Some(s)) => match std::str::from_utf8(s) {
            Ok(text) => Value::str(text),
            Err(_) => Value::str(&s.iter().map(|&b| b as char).collect::<String>()),
        },
    }
}

pub fn expr_to_value(e: &Expr) -> Value {
    expr_to_value_src(e, None)
}

/// Like [`expr_to_value`], recording the source positions of function
/// literals (for type profiling) when `src` is given as (file, text).
pub fn expr_to_value_src(e: &Expr, src: Option<(&str, &str)>) -> Value {
    let rec = |e: &Expr| expr_to_value_src(e, src);
    match e {
        Expr::Const(c, _) => const_to_value(c),
        Expr::Sym(name, _) => Value::Sym(Sym::new(name)),
        // The pipe placeholder is the string constant `"_"` in GNU R's trees.
        Expr::Placeholder(_) => Value::str("_"),
        Expr::Call { fun, args, .. } => {
            let args = args
                .iter()
                .map(|a| {
                    Arg::new(
                        a.name.as_deref().map(Sym::new),
                        a.value.as_ref().map(rec).unwrap_or_else(Value::missing_arg),
                    )
                })
                .collect();
            Value::lang(rec(fun), args)
        }
        Expr::Function { formals, body, span } => {
            let formals: Vec<Arg> = formals
                .iter()
                .map(|f| {
                    Arg::new(Some(Sym::new(&f.name)), f.default.as_ref().map(rec).unwrap_or_else(Value::missing_arg))
                })
                .collect();
            let body = rec(body);
            if let Some((file, text)) = src {
                if crate::typeprof::enabled() {
                    let (line, col) = robigo_syntax::line_col(text, span.start as usize);
                    crate::typeprof::record_function_span(&body, file, line, col);
                }
            }
            Value::lang(
                Value::Sym(s::FUNCTION),
                vec![Arg::pos(Value::pairlist(formals)), Arg::pos(body), Arg::pos(Value::Null)],
            )
        }
    }
}

/// Parse source text into an expression vector of language objects.
pub fn parse_to_values(src: &str) -> Result<Vec<Value>, robigo_syntax::ParseError> {
    let exprs = robigo_syntax::parse_program(src)?;
    Ok(exprs.iter().map(expr_to_value).collect())
}
