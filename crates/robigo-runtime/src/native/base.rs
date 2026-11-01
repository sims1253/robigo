//! The base package's registered Fortran routines (R's appl/ LINPACK QR
//! code, called by `qr()` and friends through `.Fortran(.F_xxx, ...)`).

use super::linpack::{self, Util};
use super::{NativeFn, dotc_result, int_arg, real_arg};
use crate::env::Env;
use crate::error::RResult;
use crate::interp::Interp;
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[
    ("dqrdc2", f_dqrdc2),
    ("dqrcf", |i, c, a, e| f_dqrutl(i, c, a, e, Util::Coef)),
    ("dqrqty", |i, c, a, e| f_dqrutl(i, c, a, e, Util::Qty)),
    ("dqrqy", |i, c, a, e| f_dqrutl(i, c, a, e, Util::Qy)),
    ("dqrrsd", |i, c, a, e| f_dqrutl(i, c, a, e, Util::Rsd)),
    ("dqrxb", |i, c, a, e| f_dqrutl(i, c, a, e, Util::Xb)),
    ("R_addTaskCallback", crate::task::add_task_callback),
    ("R_removeTaskCallback", crate::task::remove_task_callback),
    ("R_getTaskCallbackNames", crate::task::get_task_callback_names),
];

/// `.Fortran(.F_dqrdc2, qr = x, n, n, p, tol, rank, qraux, pivot, work)`
fn f_dqrdc2(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut x = real_arg(&args, 0);
    let ldx = int_arg(&args, 1)[0] as usize;
    let n = int_arg(&args, 2)[0] as usize;
    let p = int_arg(&args, 3)[0] as usize;
    let tol = real_arg(&args, 4)[0];
    let mut qraux = real_arg(&args, 6);
    let mut jpvt = int_arg(&args, 7);
    let mut work = real_arg(&args, 8);
    let k = linpack::dqrdc2(&mut x, ldx, n, p, tol, &mut qraux, &mut jpvt, &mut work);
    Ok(dotc_result(
        args,
        vec![
            (0, Value::reals(x)),
            (5, Value::ints(vec![k as i32])),
            (6, Value::reals(qraux)),
            (7, Value::ints(jpvt)),
            (8, Value::reals(work)),
        ],
    ))
}

/// `.Fortran(.F_dqrcf/dqrqty/dqrqy/dqrrsd/dqrxb, x, n, k, qraux, y, ny, out[, info])`
fn f_dqrutl(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env, what: Util) -> RResult<Value> {
    let mut x = real_arg(&args, 0);
    let n = int_arg(&args, 1)[0] as usize;
    let k = int_arg(&args, 2)[0] as usize;
    let qraux = real_arg(&args, 3);
    let mut y = real_arg(&args, 4);
    let ny = int_arg(&args, 5)[0] as usize;
    let mut out = real_arg(&args, 6);
    let is_coef = matches!(what, Util::Coef);
    let info = linpack::dqrutl(what, &mut x, n, k, &qraux, &mut y, ny, &mut out);
    let mut replaced = vec![(4, Value::reals(y)), (6, Value::reals(out))];
    if is_coef {
        replaced.push((7, Value::ints(vec![info])));
    }
    Ok(dotc_result(args, replaced))
}
