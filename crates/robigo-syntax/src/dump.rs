//! Canonical S-expression dump of parsed code.
//!
//! `scripts/oracle-parse.R` produces the same format from GNU R's `parse()`,
//! which lets the test suite compare both parsers tree by tree.

use crate::ast::*;
use crate::cfmt::fmt_g;

pub fn dump(e: &Expr) -> String {
    let mut s = String::new();
    dump_into(e, &mut s);
    s
}

/// Escape a string the way R's `encodeString(x, quote = '"')` does.
pub fn encode_string(x: &str, quote: char) -> String {
    let mut out = String::with_capacity(x.len() + 2);
    out.push(quote);
    for c in x.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\u{b}' => out.push_str("\\v"),
            '\\' => out.push_str("\\\\"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\{:03o}", c as u32));
            }
            // C1 control characters are not printable in a UTF-8 locale.
            c if (0x80..=0x9f).contains(&(c as u32)) => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

pub fn dump_double(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x == f64::INFINITY {
        "Inf".into()
    } else if x == f64::NEG_INFINITY {
        "-Inf".into()
    } else {
        fmt_g(x, 17, false, false)
    }
}

fn dump_const(c: &Const, s: &mut String) {
    match c {
        Const::Null => s.push_str("NULL"),
        Const::Lgl(None) => s.push_str("lgl:NA"),
        Const::Lgl(Some(b)) => s.push_str(if *b { "lgl:TRUE" } else { "lgl:FALSE" }),
        Const::Int(None) => s.push_str("int:NA"),
        Const::Int(Some(i)) => s.push_str(&format!("int:{}", i)),
        Const::DblNa => s.push_str("dbl:NA"),
        Const::Dbl(x) => {
            s.push_str("dbl:");
            s.push_str(&dump_double(*x));
        }
        Const::CplxNa => s.push_str("cplx:NA"),
        Const::Cplx(re, im) => {
            s.push_str("cplx:");
            s.push_str(&dump_double(*re));
            s.push(',');
            s.push_str(&dump_double(*im));
        }
        Const::Str(None) => s.push_str("str:NA"),
        Const::Str(Some(x)) => {
            s.push_str("str:");
            s.push_str(&encode_string(x, '"'));
        }
    }
}

fn dump_into(e: &Expr, s: &mut String) {
    match e {
        Expr::Const(c, _) => dump_const(c, s),
        Expr::Sym(name, _) => {
            s.push('`');
            s.push_str(name);
            s.push('`');
        }
        Expr::Call { fun, args, .. } => {
            s.push('(');
            dump_into(fun, s);
            for a in args {
                s.push(' ');
                dump_arg(a.name.as_deref(), a.value.as_ref(), s);
            }
            s.push(')');
        }
        Expr::Function { formals, body, .. } => {
            s.push_str("(`function` (pairlist");
            for f in formals {
                s.push(' ');
                dump_arg(Some(&f.name), f.default.as_ref(), s);
            }
            s.push_str(") ");
            dump_into(body, s);
            s.push_str(" NULL)");
        }
    }
}

fn dump_arg(name: Option<&str>, value: Option<&Expr>, s: &mut String) {
    if let Some(n) = name {
        s.push_str(n);
        s.push('=');
    }
    match value {
        Some(v) => dump_into(v, s),
        None => s.push_str("<missing>"),
    }
}
