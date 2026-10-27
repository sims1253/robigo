//! Miscellaneous internals from GNU R's util.c.

use super::internal;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static DEFS: &[Builtin] = &[internal("compareNumericVersion", do_compare_numeric_version, 2)];

// R: util.c:compareNumericVersion
fn compare_numeric_version(x: &Value, y: &Value) -> RResult<i32> {
    let Value::Int(x) = x else { return Err(Flow::error("invalid 'x' argument")) };
    let Value::Int(y) = y else { return Err(Flow::error("invalid 'y' argument")) };
    let (ix, iy) = (&x.data, &y.data);
    let nc = ix.len().min(iy.len());
    if nc == 0 {
        return Ok(NA_INTEGER);
    }
    for i in 0..nc {
        if ix[i] > iy[i] {
            return Ok(1);
        }
        if ix[i] < iy[i] {
            return Ok(-1);
        }
    }
    if ix[nc..].iter().any(|&v| v > 0) {
        return Ok(1);
    }
    if iy[nc..].iter().any(|&v| v > 0) {
        return Ok(-1);
    }
    Ok(0)
}

// R: util.c:do_compareNumericVersion
fn do_compare_numeric_version(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let elts = |v: &Value, what: &str| -> RResult<Vec<Value>> {
        match v {
            Value::List(l) => Ok(l.data.clone()),
            v if v.is_vector() => Ok(crate::coerce::to_list(v)),
            _ => Err(Flow::error(format!("invalid '{}' argument", what))),
        }
    };
    let x = elts(&args[0].value, "x")?;
    let y = elts(&args[1].value, "y")?;
    let na = if !x.is_empty() && !y.is_empty() { x.len().max(y.len()) } else { 0 };
    let mut out = Vec::with_capacity(na);
    for i in 0..na {
        out.push(compare_numeric_version(&x[i % x.len()], &y[i % y.len()])?);
    }
    Ok(Value::ints(out))
}
