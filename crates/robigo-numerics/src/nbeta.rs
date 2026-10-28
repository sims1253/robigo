//! The noncentral beta distribution: dnbeta.c, pnbeta.c, qnbeta.c.

#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::beta::dbeta;
use super::gamma::{lbeta, lgammafn};
use super::ldmath::{gt, log1pl, logl};
use super::ldouble::LDouble;
use super::pgamma::dpois_raw;
use super::toms708::bratio;
use super::*;

/// R: nmath/dnbeta.c:dnbeta
pub fn dnbeta(x: f64, a: f64, b: f64, ncp: f64, give_log: bool) -> f64 {
    let eps = 1e-15;

    if x.is_nan() || a.is_nan() || b.is_nan() || ncp.is_nan() {
        return x + a + b + ncp;
    }
    if ncp < 0.0 || a <= 0.0 || b <= 0.0 {
        return ML_NAN;
    }
    if !a.is_finite() || !b.is_finite() || !ncp.is_finite() {
        return ML_NAN;
    }
    if !(0.0..=1.0).contains(&x) {
        return r_d__0(give_log);
    }
    if ncp == 0.0 {
        return dbeta(x, a, b, give_log);
    }

    // New algorithm, starting with *largest* term:
    let ncp2 = 0.5 * ncp;
    let dx2 = ncp2 * x;
    let d = (dx2 - a - 1.0) / 2.0;
    let mut dd = d * d + dx2 * (a + b) - a;
    let k_max: i32;
    if dd <= 0.0 {
        k_max = 0;
    } else {
        dd = (d + dd.sqrt()).ceil();
        k_max = if dd > 0.0 { dd as i32 } else { 0 };
    }

    // The starting "middle term" --- first look at its log scale:
    let mut term = LDouble::from_f64(dbeta(x, a + k_max as f64, b, true));
    let p_k_d = dpois_raw(k_max as f64, ncp2, true);
    if x == 0.0 || !term.to_f64().is_finite() || !p_k_d.is_finite() {
        // if term = +Inf
        return r_d_exp((LDouble::from_f64(p_k_d) + term).to_f64(), give_log);
    }

    // Now if s_k := p_k * t_k  {here = exp(p_k + term)} would underflow,
    // we should rather scale everything and re-scale at the end:
    let p_k = LDouble::from_f64(p_k_d) + term;
    // = log(p_k) + log(t_k) == log(s_k) -- used at end to rescale
    // mid = 1 = the rescaled value, instead of  mid = exp(p_k);

    // Now sum from the inside out
    let mut sum = LDouble::from_f64(1.0); // = mid term
    term = LDouble::from_f64(1.0); // = mid term
    // middle to the left
    let mut k = k_max as f64;
    while k > 0.0 && gt(term, sum * eps) {
        k -= 1.0;
        let q = (k + 1.0) * (k + a) / (k + a + b) / dx2; // 1 / r_k
        term = term * q;
        sum = sum + term;
    }
    // middle to the right
    term = LDouble::from_f64(1.0);
    k = k_max as f64;
    loop {
        let q = dx2 * (k + a + b) / (k + a) / (k + 1.0); // r_{old k}
        k += 1.0;
        term = term * q;
        sum = sum + term;
        if !gt(term, sum * eps) {
            break;
        }
    }

    r_d_exp((p_k + logl(sum)).to_f64(), give_log)
}

/// R: nmath/pnbeta.c:pnbeta_raw
pub fn pnbeta_raw(x: f64, o_x: f64, a: f64, b: f64, ncp: f64) -> LDouble {
    // o_x == 1 - x  but maybe more accurate

    // change errmax and itrmax if desired;
    // original (AS 226, R84) had (errmax; itrmax) = (1e-6; 100)
    const ERRMAX: f64 = 1.0e-9;
    const ITRMAX: i32 = 10000; // 100 is not enough for pf(ncp=200), see PR#11277

    if ncp < 0.0 || a <= 0.0 || b <= 0.0 {
        return LDouble::from_f64(ML_NAN);
    }
    if x < 0.0 || o_x > 1.0 || (x == 0.0 && o_x == 1.0) {
        return LDouble::from_f64(0.0);
    }
    if x > 1.0 || o_x < 0.0 || (x == 1.0 && o_x == 0.0) {
        return LDouble::from_f64(1.0);
    }

    let c = ncp / 2.0;

    // initialize the series
    let x0 = fmax2(c - 7.0 * c.sqrt(), 0.0).floor();
    let a0 = a + x0;
    let l_beta = lbeta(a0, b);
    // temp = pbeta_raw(x, a0, b, TRUE, FALSE), but using (x, o_x):
    let (mut temp, _wc, _ierr) = bratio(a0, b, x, o_x, false);

    let mut gx =
        LDouble::from_f64((a0 * x.ln() + b * if x < 0.5 { log1p(-x) } else { o_x.ln() } - l_beta - a0.ln()).exp());
    let mut q = LDouble::from_f64(if a0 > a {
        // x0 >= 1 (and *not* x0 << a)
        (-c + x0 * c.ln() - lgammafn(x0 + 1.0)).exp()
    } else {
        // a0 = a  <==  x0 << a
        (-c).exp()
    });

    let mut sumq = LDouble::from_f64(1.0) - q;
    let mut ans = q * temp;

    // recurse over subsequent terms until convergence is achieved
    let mut j = x0.floor(); // x0 could be billions, and is in package EnvStats
    let mut errbd;
    loop {
        j += 1.0;
        temp -= gx.to_f64();
        gx = gx * (x * (a + b + j - 1.0) / (a + j));
        q = q * (c / j);
        sumq = sumq - q;
        let ax = LDouble::from_f64(temp) * q;
        ans = ans + ax;
        errbd = ((LDouble::from_f64(temp) - gx) * sumq).to_f64();
        if !(errbd > ERRMAX && j < ITRMAX as f64 + x0) {
            break;
        }
    }

    if errbd > ERRMAX {
        ml_warning(ME_PRECISION, "pnbeta");
    }
    if j >= ITRMAX as f64 + x0 {
        ml_warning(ME_NOCONV, "pnbeta");
    }

    ans
}

/// R: nmath/pnbeta.c:pnbeta2
pub fn pnbeta2(x: f64, o_x: f64, a: f64, b: f64, ncp: f64, lower_tail: bool, log_p: bool) -> f64 {
    // o_x == 1 - x  but maybe more accurate
    let mut ans = pnbeta_raw(x, o_x, a, b, ncp);

    // return R_DT_val(ans), but we want to warn about cancellation here
    if lower_tail {
        if log_p { logl(ans).to_f64() } else { ans.to_f64() }
    } else {
        if gt(ans, LDouble::from_f64(1.0 - 1e-10)) {
            ml_warning(ME_PRECISION, "pnbeta");
        }
        if gt(ans, LDouble::from_f64(1.0)) {
            ans = LDouble::from_f64(1.0); // Precaution
        }
        if log_p { log1pl(LDouble::from_f64(0.0) - ans).to_f64() } else { (LDouble::from_f64(1.0) - ans).to_f64() }
    }
}

/// R: nmath/pnbeta.c:pnbeta
pub fn pnbeta(x: f64, a: f64, b: f64, ncp: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || a.is_nan() || b.is_nan() || ncp.is_nan() {
        return x + a + b + ncp;
    }
    // R_P_bounds_01(x, 0., 1.)
    if x <= 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    if x >= 1.0 {
        return r_dt_1(lower_tail, log_p);
    }
    pnbeta2(x, 1.0 - x, a, b, ncp, lower_tail, log_p)
}

/// R: nmath/qnbeta.c:qnbeta
pub fn qnbeta(p: f64, a: f64, b: f64, ncp: f64, lower_tail: bool, log_p: bool) -> f64 {
    const ACCU: f64 = 1e-15;
    const EPS: f64 = 1e-14; // must be > accu

    if p.is_nan() || a.is_nan() || b.is_nan() || ncp.is_nan() {
        return p + a + b + ncp;
    }
    if !a.is_finite() {
        return ML_NAN;
    }
    if ncp < 0.0 || a <= 0.0 || b <= 0.0 {
        return ML_NAN;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, 1.0, lower_tail, log_p) {
        return v;
    }

    let p = r_dt_qiv(p, lower_tail, log_p);

    // Invert pnbeta(.):
    // 1. finding an upper and lower bound
    if p > 1.0 - DBL_EPSILON {
        return 1.0;
    }
    let mut pp = fmin2(1.0 - DBL_EPSILON, p * (1.0 + EPS));
    let mut ux = 0.5;
    while ux < 1.0 - DBL_EPSILON && pnbeta(ux, a, b, ncp, true, false) < pp {
        ux = 0.5 * (1.0 + ux);
    }
    pp = p * (1.0 - EPS);
    let mut lx = 0.5;
    while lx > DBL_MIN && pnbeta(lx, a, b, ncp, true, false) > pp {
        lx *= 0.5;
    }

    // 2. interval (lx,ux)  halving:
    loop {
        let nx = 0.5 * (lx + ux);
        if pnbeta(nx, a, b, ncp, true, false) > p {
            ux = nx;
        } else {
            lx = nx;
        }
        if !((ux - lx) / nx > ACCU) {
            break;
        }
    }

    0.5 * (ux + lx)
}
