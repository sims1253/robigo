//! `print.default` and friends (GNU R's print.c `do_printdefault`).

use super::{arg, internal};
use crate::coerce::{as_integer, as_logical};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::print::Printer;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("print.default", do_printdefault, 3).invisible(),
    internal("prmatrix", do_prmatrix, 6).invisible(),
    internal("deparse", do_deparse, 5),
    internal("dput", do_dput, 3).invisible(),
];

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

// ----------------------------------------------------------------------
// deparse.c: deparse, dput

const MIN_CUTOFF: i32 = 20;
const MAX_CUTOFF: i32 = 500;
const DEFAULT_CUTOFF: i32 = 60;

/// R: deparse.c:do_deparse
fn do_deparse(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let expr = arg(&args, 0);
    let mut cut0 = DEFAULT_CUTOFF;
    let c = arg(&args, 1);
    if !c.is_null() {
        cut0 = as_integer(&c);
        if cut0 == NA_INTEGER || !(MIN_CUTOFF..=MAX_CUTOFF).contains(&cut0) {
            i.warning("invalid 'cutoff' value for 'deparse', using default")?;
            cut0 = DEFAULT_CUTOFF;
        }
    }
    let b = arg(&args, 2);
    let backtick = if b.is_null() {
        false
    } else {
        let l = as_logical(&b);
        if l == NA_LOGICAL {
            return Err(Flow::error_call(call.clone(), "NA in coercion to boolean"));
        }
        l != 0
    };
    let o = arg(&args, 3);
    let opts = if o.is_null() { crate::deparse::SHOWATTRIBUTES } else { as_integer(&o) as u32 };
    let nlines = as_integer(&arg(&args, 4));
    let nlines = if nlines == NA_INTEGER || nlines < 0 { None } else { Some(nlines as usize) };
    let lines = crate::deparse::deparse_lines(&expr, cut0 as usize, backtick, opts, nlines);
    Ok(Value::strs_from(lines))
}

/// R: deparse.c:do_dput
fn do_dput(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let o = arg(&args, 2);
    let opts = if o.is_null() { crate::deparse::SHOWATTRIBUTES } else { as_integer(&o) as u32 };
    let tval = match &x {
        Value::Closure(c) => {
            let mut c2 = (**c).clone();
            c2.env = i.global.clone();
            Value::Closure(std::rc::Rc::new(c2))
        }
        v => v.clone(),
    };
    let lines = crate::deparse::deparse_lines(&tval, DEFAULT_CUTOFF as usize, false, opts, None);
    let file = arg(&args, 1);
    if !file.inherits("connection") {
        return Err(Flow::error("'file' must be a character string or connection"));
    }
    let ifile = as_integer(&file);
    let mut text = String::new();
    for l in &lines {
        text.push_str(l);
        text.push('\n');
    }
    if ifile == 1 {
        i.write_out(&text);
    } else {
        super::io::write_text_to(i, ifile, &text, "w")?;
    }
    Ok(x)
}

/// R: print.c:do_prmatrix
fn do_prmatrix(i: &mut Interp, _: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let mut rowlab = arg(&args, 1);
    let mut collab = arg(&args, 2);
    let quote = as_integer(&arg(&args, 3));
    let right = as_integer(&arg(&args, 4));
    let mut p = Printer::new(i, env);
    let naprint = arg(&args, 5);
    if !naprint.is_null() {
        match &naprint {
            Value::Str(v) if !v.data.is_empty() => {
                let s = v.data[0].as_deref().unwrap_or("NA").to_string();
                p.pp.na_width = crate::format::rstrlen(&s, None);
                p.pp.na_width_noquote = p.pp.na_width;
                p.pp.na_string = s.clone();
                p.pp.na_string_noquote = s;
            }
            _ => return Err(Flow::error("invalid 'na.print' specification")),
        }
    }
    if rowlab.is_empty() {
        rowlab = Value::Null;
    }
    if collab.is_empty() {
        collab = Value::Null;
    }
    if !rowlab.is_null() && !matches!(rowlab, Value::Str(_)) {
        return Err(Flow::error("invalid row labels"));
    }
    if !collab.is_null() && !matches!(collab, Value::Str(_)) {
        return Err(Flow::error("invalid column labels"));
    }
    let d = match x.get_attr(crate::sym::s::DIM) {
        Some(Value::Int(d)) => d.data.clone(),
        _ => vec![x.len() as i32, 1],
    };
    let strs = |v: &Value| v.as_str_vec().map(|s| s.data.clone());
    let (r, c) = (d[0] as usize, d.get(1).copied().unwrap_or(1) as usize);
    p.matrix(&x, r, c, strs(&rowlab), strs(&collab), None, None, quote != 0, right != 0);
    p.flush(i);
    Ok(x)
}
