//! Native routines of the stats package (src/library/stats/src).

mod cov;

use super::NativeFn;
use crate::error::RResult;
use crate::interp::Interp;
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[("cov", c_cov), ("cor", c_cor)];

fn c_cov(i: &mut Interp, _: &Value, a: Vec<Arg>) -> RResult<Value> {
    cov::corcov(i, &a[0].value, &a[1].value, &a[2].value, &a[3].value, false)
}

fn c_cor(i: &mut Interp, _: &Value, a: Vec<Arg>) -> RResult<Value> {
    cov::corcov(i, &a[0].value, &a[1].value, &a[2].value, &a[3].value, true)
}
