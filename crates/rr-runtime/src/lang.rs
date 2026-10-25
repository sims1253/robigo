//! Conversion from syntax trees to R language objects.

use rr_syntax::{Const, Expr};

use crate::sym::{s, Sym};
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
        Const::Str(Some(s)) => Value::str(s),
    }
}

pub fn expr_to_value(e: &Expr) -> Value {
    match e {
        Expr::Const(c, _) => const_to_value(c),
        Expr::Sym(name, _) => Value::Sym(Sym::new(name)),
        Expr::Call { fun, args, .. } => {
            let args = args
                .iter()
                .map(|a| {
                    Arg::new(
                        a.name.as_deref().map(Sym::new),
                        a.value.as_ref().map(expr_to_value).unwrap_or_else(Value::missing_arg),
                    )
                })
                .collect();
            Value::lang(expr_to_value(fun), args)
        }
        Expr::Function { formals, body, .. } => {
            let formals: Vec<Arg> = formals
                .iter()
                .map(|f| {
                    Arg::new(
                        Some(Sym::new(&f.name)),
                        f.default.as_ref().map(expr_to_value).unwrap_or_else(Value::missing_arg),
                    )
                })
                .collect();
            Value::lang(
                Value::Sym(s::FUNCTION),
                vec![Arg::pos(Value::pairlist(formals)), Arg::pos(expr_to_value(body)), Arg::pos(Value::Null)],
            )
        }
    }
}

/// Parse source text into an expression vector of language objects.
pub fn parse_to_values(src: &str) -> Result<Vec<Value>, rr_syntax::ParseError> {
    let exprs = rr_syntax::parse_program(src)?;
    Ok(exprs.iter().map(expr_to_value).collect())
}
