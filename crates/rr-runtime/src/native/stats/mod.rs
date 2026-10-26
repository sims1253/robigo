//! Native routines of the stats package (src/library/stats/src).

mod cov;
mod lm;
mod model;

use super::NativeFn;
use crate::env::Env;
use crate::error::RResult;
use crate::interp::Interp;
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[
    ("cov", c_cov),
    ("cor", c_cor),
    ("termsform", |i, _, a, _| model::termsform(i, &a)),
    ("modelframe", |i, _, a, env| model::modelframe(i, &a, env)),
    ("modelmatrix", |i, _, a, env| model::modelmatrix(i, &a, env)),
    ("updateform", |_, _, a, _| model::updateform(&a[0].value, &a[1].value)),
    ("Cdqrls", |i, _, a, _| lm::cdqrls(i, &a[0].value, &a[1].value, &a[2].value, &a[3].value)),
];

fn c_cov(i: &mut Interp, _: &Value, a: Vec<Arg>, _: &Env) -> RResult<Value> {
    cov::corcov(i, &a[0].value, &a[1].value, &a[2].value, &a[3].value, false)
}

fn c_cor(i: &mut Interp, _: &Value, a: Vec<Arg>, _: &Env) -> RResult<Value> {
    cov::corcov(i, &a[0].value, &a[1].value, &a[2].value, &a[3].value, true)
}
