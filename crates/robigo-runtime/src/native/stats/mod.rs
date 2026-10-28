//! Native routines of the stats package (src/library/stats/src).

mod cov;
mod distn;
mod family;
mod lm;
mod misc;
mod model;

use super::NativeFn;
use crate::env::Env;
use crate::error::RResult;
use crate::interp::Interp;
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[
    ("dnbeta", |i, _, a, _| distn::dnbeta(i, &a)),
    ("pnbeta", |i, _, a, _| distn::pnbeta(i, &a)),
    ("qnbeta", |i, _, a, _| distn::qnbeta(i, &a)),
    ("dnf", |i, _, a, _| distn::dnf(i, &a)),
    ("pnf", |i, _, a, _| distn::pnf(i, &a)),
    ("qnf", |i, _, a, _| distn::qnf(i, &a)),
    ("dnt", |i, _, a, _| distn::dnt(i, &a)),
    ("pnt", |i, _, a, _| distn::pnt(i, &a)),
    ("qnt", |i, _, a, _| distn::qnt(i, &a)),
    ("dnchisq", |i, _, a, _| distn::dnchisq(i, &a)),
    ("pnchisq", |i, _, a, _| distn::pnchisq(i, &a)),
    ("qnchisq", |i, _, a, _| distn::qnchisq(i, &a)),
    ("cov", c_cov),
    ("cor", c_cor),
    ("termsform", |i, _, a, _| model::termsform(i, &a)),
    ("modelframe", |i, _, a, env| model::modelframe(i, &a, env)),
    ("modelmatrix", |i, _, a, env| model::modelmatrix(i, &a, env)),
    ("updateform", |_, _, a, _| model::updateform(&a[0].value, &a[1].value)),
    ("logit_link", |i, _, a, _| family::logit_link(i, &a[0].value)),
    ("logit_linkinv", |i, _, a, _| family::logit_linkinv(i, &a[0].value)),
    ("logit_mu_eta", |i, _, a, _| family::logit_mu_eta(i, &a[0].value)),
    ("binomial_dev_resids", |i, _, a, _| family::binomial_dev_resids(i, &a[0].value, &a[1].value, &a[2].value)),
    ("ApproxTest", |_, _, a, _| misc::approx_test(&a)),
    ("Approx", |i, _, a, _| misc::approx(i, &a)),
    ("compcases", |_, _, a, _| misc::compcases(&a)),
    ("Cdist", |i, _, a, _| misc::cdist(i, &a)),
    ("cfilter", |_, _, a, _| misc::cfilter(&a)),
    ("rfilter", |_, _, a, _| misc::rfilter(&a)),
    ("acf", |i, _, a, _| misc::acf(i, &a)),
    ("do_fmin", |i, _, a, env| misc::do_fmin(i, &a, env)),
    ("zeroin2", |i, _, a, env| misc::zeroin2(i, &a, env)),
    ("Cdqrls", |i, _, a, _| lm::cdqrls(i, &a[0].value, &a[1].value, &a[2].value, &a[3].value)),
];

fn c_cov(i: &mut Interp, _: &Value, a: Vec<Arg>, _: &Env) -> RResult<Value> {
    cov::corcov(i, &a[0].value, &a[1].value, &a[2].value, &a[3].value, false)
}

fn c_cor(i: &mut Interp, _: &Value, a: Vec<Arg>, _: &Env) -> RResult<Value> {
    cov::corcov(i, &a[0].value, &a[1].value, &a[2].value, &a[3].value, true)
}
