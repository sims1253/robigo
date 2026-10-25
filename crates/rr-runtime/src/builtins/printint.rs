//! `print.default` and friends (GNU R's print.c `do_printdefault`).

use super::{arg, internal};
use crate::coerce::{as_integer, as_logical};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::print::Printer;
use crate::value::*;

pub static DEFS: &[Builtin] = &[internal("print.default", do_printdefault, 3).invisible()];

fn do_printdefault(i: &mut Interp, _: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let pargs: Vec<Arg> = match arg(&args, 1) {
        Value::Pairlist(p) => p.items.clone(),
        _ => vec![],
    };
    let missing: Vec<bool> = match arg(&args, 2) {
        Value::Lgl(v) => v.data.iter().map(|&b| b != 0).collect(),
        _ => vec![],
    };
    let mut p = Printer::new(i, env);
    let get = |k: usize| pargs.get(k).map(|a| a.value.clone()).unwrap_or(Value::Null);
    // digits
    let digits = get(0);
    if !digits.is_null() {
        let d = as_integer(&digits);
        if d == NA_INTEGER || !(1..=22).contains(&d) {
            return Err(Flow::error("invalid digits parameter"));
        }
        p.pp.digits = d as usize;
    }
    let q = as_logical(&get(1));
    if q == NA_LOGICAL {
        return Err(Flow::error("invalid 'quote' argument"));
    }
    p.pp.quote = q != 0;
    let na = get(2);
    if !na.is_null() {
        match &na {
            Value::Str(v) if !v.data.is_empty() => {
                let s = v.data[0].as_deref().unwrap_or("NA").to_string();
                p.pp.na_width = crate::format::str_width(&s);
                p.pp.na_width_noquote = p.pp.na_width;
                p.pp.na_string = s.clone();
                p.pp.na_string_noquote = s;
            }
            _ => return Err(Flow::error("invalid 'na.print' specification")),
        }
    }
    let gap = get(3);
    if !gap.is_null() {
        let g = as_integer(&gap);
        if g == NA_INTEGER || g < 0 {
            return Err(Flow::error("'gap' must be non-negative integer"));
        }
        p.pp.gap = g as usize;
    }
    let right = as_logical(&get(4));
    if right == NA_LOGICAL {
        return Err(Flow::error("invalid 'right' argument"));
    }
    p.pp.right = right != 0;
    let max = get(5);
    if !max.is_null() {
        let m = as_integer(&max);
        if m == NA_INTEGER || m < 0 {
            return Err(Flow::error("invalid 'max' argument"));
        }
        p.pp.max = m as usize;
    }
    let width = get(6);
    if !width.is_null() {
        let w = as_integer(&width);
        if w == NA_INTEGER || w < 10 {
            return Err(Flow::error("invalid 'width' argument"));
        }
        p.pp.width = w as usize;
    }
    // Forward user-supplied arguments to print() for objects inside x.
    let mut forward = Vec::new();
    for (k, a) in pargs.iter().enumerate() {
        if k < 8 && missing.get(k).copied().unwrap_or(true) {
            continue;
        }
        forward.push(a.clone());
    }
    p.call_args = forward;
    p.value_rec(i, &x)?;
    p.flush(i);
    Ok(x)
}
