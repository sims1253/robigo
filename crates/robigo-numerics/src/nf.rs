//! The noncentral F distribution: dnf.c, pnf.c, qnf.c.

use super::nbeta::{pnbeta2, qnbeta};
use super::nchisq::{dnchisq, qnchisq};
use super::pgamma::dgamma;
use super::*;

/// R: nmath/dnf.c:dnf
pub fn dnf(x: f64, df1: f64, df2: f64, ncp: f64, give_log: bool) -> f64 {
    if x.is_nan() || df1.is_nan() || df2.is_nan() || ncp.is_nan() {
        return x + df2 + df1 + ncp;
    }

    // want to compare dnf(ncp=0) behavior with df() one, hence *NOT* :
    // if (ncp == 0) return df(x, df1, df2, give_log);

    if df1 <= 0.0 || df2 <= 0.0 || ncp < 0.0 {
        return ML_NAN;
    }
    if x < 0.0 {
        return r_d__0(give_log);
    }
    if !ncp.is_finite() {
        // ncp = +Inf -- FIXME?: in some cases, limit exists
        return ML_NAN;
    }

    if !df1.is_finite() && !df2.is_finite() {
        // both +Inf
        // PR: not sure about this (taken from ncp==0) -- FIXME ?
        return if x == 1.0 { ML_POSINF } else { r_d__0(give_log) };
    }
    if !df2.is_finite() {
        // i.e. = +Inf
        return df1 * dnchisq(x * df1, df1, ncp, give_log);
    }
    // == dngamma(x, df1/2, 2./df1, ncp, give_log) -- but that does not exist
    if df1 > 1e14 && ncp < 1e7 {
        // includes df1 == +Inf: code below is inaccurate there
        let f = 1.0 + ncp / df1; // assumes ncp << df1 [ignores 2*ncp^(1/2)/df1*x term]
        let z = dgamma(1.0 / x / f, df2 / 2.0, 2.0 / df2, give_log);
        return if give_log { z - 2.0 * x.ln() - f.ln() } else { z / (x * x) / f };
    }

    let y = (df1 / df2) * x;
    let z = super::nbeta::dnbeta(y / (1.0 + y), df1 / 2.0, df2 / 2.0, ncp, give_log);
    if give_log { z + df1.ln() - df2.ln() - 2.0 * log1p(y) } else { z * (df1 / df2) / (1.0 + y) / (1.0 + y) }
}

/// R: nmath/pnf.c:pnf
pub fn pnf(x: f64, df1: f64, df2: f64, ncp: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || df1.is_nan() || df2.is_nan() || ncp.is_nan() {
        return x + df2 + df1 + ncp;
    }
    if df1 <= 0.0 || df2 <= 0.0 || ncp < 0.0 {
        return ML_NAN;
    }
    if !ncp.is_finite() {
        return ML_NAN;
    }
    if !df1.is_finite() && !df2.is_finite() {
        // both +Inf
        return ML_NAN;
    }

    // R_P_bounds_01(x, 0., ML_POSINF)
    if x <= 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    if x >= ML_POSINF {
        return r_dt_1(lower_tail, log_p);
    }

    if df2 > 1e8 {
        // avoid problems with +Inf and loss of accuracy
        return super::nchisq::pnchisq(x * df1, df1, ncp, lower_tail, log_p);
    }

    let y = (df1 / df2) * x;
    pnbeta2(y / (1.0 + y), 1.0 / (1.0 + y), df1 / 2.0, df2 / 2.0, ncp, lower_tail, log_p)
}

/// R: nmath/qnf.c:qnf
pub fn qnf(p: f64, df1: f64, df2: f64, ncp: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || df1.is_nan() || df2.is_nan() || ncp.is_nan() {
        return p + df1 + df2 + ncp;
    }
    if df1 <= 0.0 || df2 <= 0.0 || ncp < 0.0 {
        return ML_NAN;
    }
    if !ncp.is_finite() {
        return ML_NAN;
    }
    if !df1.is_finite() && !df2.is_finite() {
        return ML_NAN;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, ML_POSINF, lower_tail, log_p) {
        return v;
    }

    if df2 > 1e8 {
        // avoid problems with +Inf and loss of accuracy
        return qnchisq(p, df1, ncp, lower_tail, log_p) / df1;
    }

    let y = qnbeta(p, df1 / 2.0, df2 / 2.0, ncp, lower_tail, log_p);
    y / (1.0 - y) * (df2 / df1)
}
