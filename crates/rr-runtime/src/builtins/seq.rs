//! Sequences (GNU R's seq.c). Seeded with `:`, `seq_len` and `seq_along`.

use super::{check_arity, prim};
use crate::coerce::as_real;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim(":", do_colon, 2),
    prim("seq_len", do_seq_len, 1),
    prim("seq_along", do_seq_along, 1),
];

fn do_colon(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 2, ":", call)?;
    let (x, y) = (&args[0].value, &args[1].value);
    if x.inherits("factor") && y.inherits("factor") {
        let f = i.find_fun(crate::sym::Sym::new("interaction"), &i.base.clone())?;
        return i.call_value(&f, vec![Arg::pos(x.clone()), Arg::pos(y.clone()), Arg::new(Some(crate::sym::Sym::new("sep")), Value::str(":"))], env);
    }
    let check = |v: &Value, which: &str| -> RResult<f64> {
        if v.len() != 1 {
            if v.is_empty() {
                return Err(Flow::error_call(call.clone(), "argument of length 0"));
            }
            // R warns and uses the first element.
            let _ = which;
        }
        let d = as_real(v);
        if d.is_nan() {
            return Err(Flow::error_call(call.clone(), "NA/NaN argument"));
        }
        Ok(d)
    };
    let n1 = check(x, "from")?;
    let n2 = check(y, "to")?;
    if x.len() > 1 {
        i.warningcall(call, format!("numerical expression has {} elements: only the first used", x.len()))?;
    }
    if y.len() > 1 {
        i.warningcall(call, format!("numerical expression has {} elements: only the first used", y.len()))?;
    }
    seq_colon(n1, n2, call)
}

/// R's `seq_colon`.
pub fn seq_colon(n1: f64, n2: f64, call: &Value) -> RResult<Value> {
    let r = (n2 - n1).abs();
    if r >= i32::MAX as f64 * 2.0 {
        return Err(Flow::error_call(call.clone(), "result would be too long a vector"));
    }
    let n = (r + 1.0 + f64::EPSILON) as usize;
    let use_int = n1 <= i32::MAX as f64 && n1 >= i32::MIN as f64 && n1 == n1.trunc() && {
        let last = if n1 <= n2 { n1 + (n - 1) as f64 } else { n1 - (n - 1) as f64 };
        last <= i32::MAX as f64 && last >= i32::MIN as f64
    };
    if use_int {
        let start = n1 as i64;
        let v: Vec<i32> = if n1 <= n2 {
            (0..n as i64).map(|k| (start + k) as i32).collect()
        } else {
            (0..n as i64).map(|k| (start - k) as i32).collect()
        };
        Ok(Value::ints(v))
    } else {
        let v: Vec<f64> = if n1 <= n2 {
            (0..n).map(|k| n1 + k as f64).collect()
        } else {
            (0..n).map(|k| n1 - k as f64).collect()
        };
        Ok(Value::reals(v))
    }
}

fn do_seq_len(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "seq_len", call)?;
    let v = &args[0].value;
    if v.len() != 1 {
        i.warningcall(call, format!("first element used of '{}' argument", "length.out"))?;
    }
    let d = as_real(v);
    if d.is_nan() || d < 0.0 {
        return Err(Flow::error_call(call.clone(), "argument of length 0"))
            .or_else(|_: Flow| Err(Flow::error_call(call.clone(), "argument must be coercible to non-negative integer")));
    }
    let n = d as usize;
    Ok(Value::ints((1..=n as i32).collect()))
}

fn do_seq_along(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "seq_along", call)?;
    let n = if args[0].value.is_object() {
        let f = i.find_fun(crate::sym::Sym::new("length"), &i.base.clone())?;
        let r = i.call_value(&f, vec![Arg::pos(args[0].value.clone())], env)?;
        crate::coerce::as_integer(&r).max(0) as usize
    } else {
        args[0].value.len()
    };
    Ok(Value::ints((1..=n as i32).collect()))
}
