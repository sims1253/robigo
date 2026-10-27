//! Parsing and deparsing builtins: `parse`, `str2lang`, `str2expression`,
//! `deparse`, `dput`, plus `expression`, `all.names`, `withVisible`
//! (GNU R's source.c, deparse.c, list.c, builtin.c, eval.c).

use std::rc::Rc;

use super::{arg, internal, special};
use crate::deparse::{DEFAULTDEPARSE, deparse_lines};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::lang::expr_to_value;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("parse", do_parse, 6),
    internal("str2lang", do_str2lang, 1),
    internal("str2expression", do_str2expression, 1),
    internal("deparse", do_deparse, 5),
    internal("dput", do_dput, 3).invisible(),
    special("expression", do_expression, -1),
    internal("all.names", do_allnames, 4),
    internal("withVisible", do_withvisible, 1).special(),
];

/// R's parse error message: location, the offending line(s) and a caret.
pub fn format_parse_error(src: &str, srcname: &str, e: &robigo_syntax::ParseError) -> String {
    let (line, col) = robigo_syntax::line_col(src, e.offset);
    let (_, end_col) = robigo_syntax::line_col(src, e.end.max(e.offset + 1) - 1);
    let mut msg = format!("{}:{}:{}: {}", srcname, line, col, e.message);
    let lines: Vec<&str> = src.split('\n').collect();
    let width = line.to_string().len() + 2;
    if line >= 2 {
        if let Some(prev) = lines.get(line - 2) {
            msg.push_str(&format!("\n{}: {}", line - 1, prev));
        }
    }
    if let Some(cur) = lines.get(line - 1) {
        // The context line stops after the offending token.
        let upto: String = cur.chars().take(end_col).collect();
        msg.push_str(&format!("\n{}: {}", line, upto));
        let caret_col = if col == 1 { 0 } else { end_col };
        msg.push_str(&format!("\n{}^", " ".repeat(width + caret_col)));
    }
    msg
}

pub fn parse_text(src: &str, srcname: &str) -> RResult<Vec<Value>> {
    match robigo_syntax::parse_program(src) {
        Ok(exprs) => Ok(exprs.iter().map(expr_to_value).collect()),
        Err(e) => Err(Flow::error(format_parse_error(src, srcname, &e))),
    }
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.data.iter().map(|x| x.as_deref().unwrap_or("NA")).collect::<Vec<_>>().join("\n")),
        _ => None,
    }
}

fn do_parse(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = arg(&args, 1);
    let text = arg(&args, 2);
    let srcfile = match arg(&args, 4) {
        Value::Str(s) => s.data.first().cloned().flatten().map(|s| s.to_string()).unwrap_or_else(|| "<text>".into()),
        _ => "<text>".into(),
    };
    let src = match text_of(&text) {
        Some(src) => src,
        None if text.is_null() => {
            // R: source.c:do_parse, reading the whole connection
            use super::io;
            let con = io::connection_number(&arg(&args, 0))?;
            if con < 3 {
                return Err(Flow::error("robigo: parse() from the console is not implemented yet"));
            }
            let (wasopen, ..) = io::conn_state(i, con)?;
            if !wasopen && !io::open_in_mode(i, con, "r")? {
                return Err(Flow::error("cannot open the connection"));
            }
            let r = (|| {
                if !io::conn_state(i, con)?.1 {
                    return Err(Flow::error("cannot read from this connection"));
                }
                io::con_read_all(i, con)
            })();
            if !wasopen {
                io::close_conn(i, con)?;
            }
            String::from_utf8_lossy(&r?).into_owned()
        }
        None => return Err(Flow::error("coercion of 'text' to character was unsuccessful")),
    };
    let mut exprs = parse_text(&src, &srcfile)?;
    if !n.is_null() {
        let k = crate::coerce::as_integer(&n);
        if k != NA_INTEGER && k >= 0 && (k as usize) < exprs.len() {
            exprs.truncate(k as usize);
        }
    }
    Ok(Value::expr_vec(exprs))
}

fn do_str2lang(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let s = match arg(&args, 0) {
        Value::Str(s) => s,
        _ => return Err(Flow::error("argument must be a character string")),
    };
    if s.data.len() != 1 {
        return Err(Flow::error(format!("argument must be a character string")));
    }
    let src = s.data[0].as_deref().unwrap_or("NA");
    let exprs = parse_text(src, "<text>")?;
    if exprs.len() != 1 {
        return Err(Flow::error(format!("parsing result not of length one, but {}", exprs.len())));
    }
    Ok(exprs.into_iter().next().unwrap())
}

fn do_str2expression(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let Some(src) = text_of(&arg(&args, 0)) else {
        return Err(Flow::error("argument must be a character string"));
    };
    Ok(Value::expr_vec(parse_text(&src, "<text>")?))
}

fn do_deparse(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let expr = arg(&args, 0);
    let cutoff = crate::coerce::as_integer(&arg(&args, 1));
    if cutoff == NA_INTEGER || !(20..=500).contains(&cutoff) {
        return Err(Flow::error("invalid 'cutoff' value for 'deparse', using default"));
    }
    let backtick = crate::coerce::as_logical(&arg(&args, 2)) == TRUE;
    let opts = crate::coerce::as_integer(&arg(&args, 3));
    let opts = if opts == NA_INTEGER { DEFAULTDEPARSE } else { opts as u32 };
    let nlines = crate::coerce::as_integer(&arg(&args, 4));
    let nlines = if nlines == NA_INTEGER || nlines < 0 { None } else { Some(nlines as usize) };
    let lines = deparse_lines(&expr, cutoff as usize, backtick, opts, nlines);
    Ok(Value::strs_from(lines))
}

fn do_dput(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let con = crate::coerce::as_integer(&arg(&args, 1));
    let opts = crate::coerce::as_integer(&arg(&args, 2));
    let opts = if opts == NA_INTEGER { DEFAULTDEPARSE } else { opts as u32 };
    let mut text = String::new();
    for l in deparse_lines(&x, 70, true, opts, None) {
        text.push_str(&l);
        text.push('\n');
    }
    crate::builtins::io::write_connection(i, con, &text)?;
    Ok(x)
}

/// `expression(...)`: unevaluated arguments as an expression vector.
fn do_expression(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let any_tag = args.iter().any(|a| a.tag.is_some());
    let names: Vec<RStr> = args.iter().map(|a| Some(a.tag.map(|t| t.name()).unwrap_or_else(|| Rc::from("")))).collect();
    let mut v = Value::expr_vec(args.into_iter().map(|a| a.value).collect());
    if any_tag {
        v.set_attr_raw(s::NAMES, Value::strs(names));
    }
    Ok(v)
}

fn all_names_rec(e: &Value, functions: bool, unique: bool, max: i64, out: &mut Vec<Rc<str>>) {
    if max >= 0 && out.len() as i64 >= max {
        return;
    }
    match e {
        Value::Sym(s) => {
            let n = s.name();
            if n.is_empty() {
                return;
            }
            if unique && out.iter().any(|x| *x == n) {
                return;
            }
            out.push(n);
        }
        Value::Lang(l) => {
            if functions {
                all_names_rec(&l.fun, functions, unique, max, out);
            } else if !matches!(l.fun, Value::Sym(_)) {
                all_names_rec(&l.fun, functions, unique, max, out);
            }
            for a in &l.args {
                all_names_rec(&a.value, functions, unique, max, out);
            }
        }
        Value::Expr(v) => {
            for x in &v.data {
                all_names_rec(x, functions, unique, max, out);
            }
        }
        _ => {}
    }
}

fn do_allnames(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let expr = arg(&args, 0);
    let functions = crate::coerce::as_logical(&arg(&args, 1)) == TRUE;
    let max = crate::coerce::as_integer(&arg(&args, 2));
    let max = if max == NA_INTEGER { -1 } else { max as i64 };
    let unique = crate::coerce::as_logical(&arg(&args, 3)) == TRUE;
    let mut out = Vec::new();
    all_names_rec(&expr, functions, unique, max, &mut out);
    Ok(Value::strs(out.into_iter().map(Some).collect()))
}

fn do_withvisible(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.len() != 1 {
        return Err(Flow::error_call(call.clone(), "withVisible requires one argument"));
    }
    let v = i.eval(&args[0].value, env)?;
    let vis = i.visible;
    i.visible = true;
    let names = Value::strs_from(["value", "visible"]);
    let mut l = Value::list(vec![v, Value::lgl(vis)]);
    l.set_attr_raw(s::NAMES, names);
    let _ = Sym::new("value");
    Ok(l)
}
