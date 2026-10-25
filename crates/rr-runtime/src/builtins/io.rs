//! Input/output: connections and `cat` (GNU R's connections.c and
//! builtin.c `do_cat`). Only the standard terminal connections exist so
//! far.

use std::rc::Rc;

use super::{arg, internal};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::format::{encode_logical, encode_real0, format_real, encode_integer};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("cat", do_cat, 6).invisible(),
    internal("stdin", do_stdin, 0),
    internal("stdout", do_stdout, 0),
    internal("stderr", do_stderr, 0),
];

/// A terminal connection object: integer with class c("terminal", "connection").
pub fn terminal_connection(n: i32) -> Value {
    let mut v = Value::int(n);
    v.set_attr_raw(s::CLASS, Value::strs_from(["terminal", "connection"]));
    v
}

fn do_stdin(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(terminal_connection(0))
}

fn do_stdout(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(terminal_connection(1))
}

fn do_stderr(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(terminal_connection(2))
}

/// Write to a connection number (1 = stdout, 2 = stderr).
pub fn write_connection(i: &mut Interp, con: i32, text: &str) -> RResult<()> {
    match con {
        1 => {
            i.write_out(text);
            Ok(())
        }
        2 => {
            i.write_err(text);
            Ok(())
        }
        _ => Err(Flow::error("cannot write to this connection")),
    }
}

/// `EncodeElement0(x, i, quote = 0)` as used by cat.
pub fn encode_element_cat(i: &Interp, v: &Value, k: usize) -> String {
    let pp = i.print_params();
    match v {
        Value::Lgl(x) => encode_logical(x.data[k], 0, "NA"),
        Value::Int(x) => encode_integer(x.data[k], 0, "NA"),
        Value::Real(x) => {
            let (w, d, e) = format_real(&x.data[k..k + 1], pp.digits, pp.scipen, 0, pp.na_width);
            encode_real0(x.data[k], w, d, e, "NA")
        }
        Value::Cplx(x) => {
            let z = x.data[k];
            let (r, im) = crate::format::format_complex(&[z], pp.digits, pp.scipen, 0, pp.na_width);
            crate::format::encode_complex(z, r, im, "NA")
        }
        Value::Str(x) => x.data[k].as_deref().unwrap_or("NA").to_string(),
        Value::Raw(x) => format!("{:02x}", x.data[k]),
        _ => String::new(),
    }
}

fn do_cat(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let objs = match arg(&args, 0) {
        Value::List(l) => l.data.clone(),
        _ => vec![],
    };
    let con = crate::coerce::as_integer(&arg(&args, 1));
    let sepr: Vec<String> = match arg(&args, 2) {
        Value::Str(v) => v.data.iter().map(|x| x.as_deref().unwrap_or("NA").to_string()).collect(),
        _ => return Err(Flow::error("invalid 'sep' specification")),
    };
    let nlsep = sepr.iter().any(|x| x.contains('\n'));
    let fill = arg(&args, 3);
    let pwidth: usize = match &fill {
        Value::Lgl(_) => {
            if crate::coerce::as_logical(&fill) == TRUE {
                i.print_params().width
            } else {
                usize::MAX
            }
        }
        Value::Int(_) | Value::Real(_) => {
            let w = crate::coerce::as_integer(&fill);
            if w <= 0 {
                i.warning("non-positive 'fill' argument will be ignored")?;
                usize::MAX
            } else {
                w as usize
            }
        }
        _ => return Err(Flow::error("invalid 'fill' argument")),
    };
    let labs: Vec<String> = match arg(&args, 4) {
        Value::Str(v) => v.data.iter().map(|x| x.as_deref().unwrap_or("NA").to_string()).collect(),
        Value::Null => vec![],
        _ => return Err(Flow::error("invalid 'labels' argument")),
    };
    let sep_at = |k: usize| -> &str { if sepr.is_empty() { "" } else { &sepr[k % sepr.len()] } };
    let mut out = String::new();
    let mut width = 0usize;
    let mut ntot = 0usize;
    let mut nlines = 0usize;
    let newline = |out: &mut String, width: &mut usize, nlines: usize| {
        out.push('\n');
        *width = 0;
        if !labs.is_empty() {
            let l = &labs[nlines % labs.len()];
            out.push_str(l);
            out.push(' ');
            *width += l.chars().count() + 1;
        }
    };
    for (iobj, s0) in objs.iter().enumerate() {
        if iobj != 0 && !s0.is_null() {
            out.push_str(sep_at(ntot));
            ntot += 1;
        }
        let n = s0.len();
        if n == 0 {
            continue;
        }
        if !labs.is_empty() && iobj == 0 && crate::coerce::as_integer(&fill) > 0 {
            let l = &labs[nlines % labs.len()];
            out.push_str(l);
            out.push(' ');
            width += l.chars().count() + 1;
            nlines += 1;
        }
        let elem = |k: usize| -> RResult<String> {
            match s0 {
                Value::Sym(sy) => Ok(sy.name().to_string()),
                v if v.is_atomic() => Ok(encode_element_cat(i, v, k)),
                v => Err(Flow::error(format!(
                    "argument {} (type '{}') cannot be handled by 'cat'",
                    iobj + 1,
                    v.type_name()
                ))),
            }
        };
        let mut p = elem(0)?;
        let mut w = p.len();
        let mut sepw = sep_at(ntot).len();
        if iobj > 0 && width + w + sepw > pwidth {
            newline(&mut out, &mut width, nlines);
            nlines += 1;
        }
        for k in 0..n {
            out.push_str(&p);
            width += w + sepw;
            if k < n - 1 {
                out.push_str(sep_at(ntot));
                p = elem(k + 1)?;
                w = p.len();
                sepw = sep_at(ntot).len();
                if width + w + sepw > pwidth {
                    newline(&mut out, &mut width, nlines);
                    nlines += 1;
                }
                ntot += 1;
            }
        }
    }
    if pwidth != usize::MAX || nlsep {
        out.push('\n');
    }
    let _ = call;
    write_connection(i, con, &out)?;
    Ok(Value::Null)
}

#[allow(dead_code)]
fn unused() -> Rc<str> {
    Rc::from("")
}
