//! The negative binomial distribution: dnbinom.c (dnbinom, dnbinom_mu),
//! pnbinom.c (pnbinom, pnbinom_mu), qnbinom.c, qnbinom_mu.c.

use super::beta::pbeta;
use super::binom::{dbinom_raw, discrete_quantile};
use super::pgamma::{dpois_raw, lgamma1p, ppois, qpois};
use super::toms708::bratio;
use super::*;

/// R: nmath/dnbinom.c:dnbinom
pub fn dnbinom(x: f64, size: f64, prob: f64, give_log: bool) -> f64 {
    if x.is_nan() || size.is_nan() || prob.is_nan() {
        return x + size + prob;
    }
    if prob <= 0.0 || prob > 1.0 || size < 0.0 {
        return ML_NAN;
    }
    if let Some(v) = r_d_nonint_check(x, give_log) {
        return v;
    }
    if x < 0.0 || !x.is_finite() {
        return r_d__0(give_log);
    }
    let x = r_forceint(x);
    if x == 0.0 {
        if size == 0.0 {
            return r_d__1(give_log);
        }
        return if give_log { size * prob.ln() } else { prob.powf(size) };
    }
    let size = if !size.is_finite() { DBL_MAX } else { size };
    if x < 1e-10 * size {
        r_d_exp(
            size * prob.ln() + x * (size.ln() + log1p(-prob)) - lgamma1p(x) + log1p(x * (x - 1.0) / (2.0 * size)),
            give_log,
        )
    } else {
        let p = if give_log {
            if x < size { log1p(-x / (size + x)) } else { (size / (size + x)).ln() }
        } else {
            size / (size + x)
        };
        let ans = dbinom_raw(size, x + size, prob, 1.0 - prob, give_log);
        if give_log { p + ans } else { p * ans }
    }
}

/// R: nmath/dnbinom.c:dnbinom_mu
pub fn dnbinom_mu(x: f64, size: f64, mu: f64, give_log: bool) -> f64 {
    if x.is_nan() || size.is_nan() || mu.is_nan() {
        return x + size + mu;
    }
    if mu < 0.0 || size < 0.0 {
        return ML_NAN;
    }
    if let Some(v) = r_d_nonint_check(x, give_log) {
        return v;
    }
    if x < 0.0 || !x.is_finite() {
        return r_d__0(give_log);
    }
    if x == 0.0 && size == 0.0 {
        return r_d__1(give_log);
    }
    let x = r_forceint(x);
    if !size.is_finite() {
        return dpois_raw(x, mu, give_log);
    }
    if x == 0.0 {
        return r_d_exp(
            size * (if size < mu { (size / (size + mu)).ln() } else { log1p(-mu / (size + mu)) }),
            give_log,
        );
    }
    if x < 1e-10 * size {
        let p = if size < mu { (size / (1.0 + size / mu)).ln() } else { (mu / (1.0 + mu / size)).ln() };
        r_d_exp(x * p - mu - lgamma1p(x) + log1p(x * (x - 1.0) / (2.0 * size)), give_log)
    } else {
        let p = if give_log {
            if x < size { log1p(-x / (size + x)) } else { (size / (size + x)).ln() }
        } else {
            size / (size + x)
        };
        let ans = dbinom_raw(size, x + size, size / (size + mu), mu / (size + mu), give_log);
        if give_log { p + ans } else { p * ans }
    }
}

/// R: nmath/pnbinom.c:pnbinom
pub fn pnbinom(x: f64, size: f64, prob: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || size.is_nan() || prob.is_nan() {
        return x + size + prob;
    }
    if !size.is_finite() || !prob.is_finite() {
        return ML_NAN;
    }
    if size < 0.0 || prob <= 0.0 || prob > 1.0 {
        return ML_NAN;
    }
    if size == 0.0 {
        return if x >= 0.0 { r_dt_1(lower_tail, log_p) } else { r_dt_0(lower_tail, log_p) };
    }
    if x < 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    if !x.is_finite() {
        return r_dt_1(lower_tail, log_p);
    }
    let x = (x + 1e-7).floor();
    pbeta(prob, size, x + 1.0, lower_tail, log_p)
}

/// R: nmath/pnbinom.c:pnbinom_mu
pub fn pnbinom_mu(x: f64, size: f64, mu: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || size.is_nan() || mu.is_nan() {
        return x + size + mu;
    }
    if !mu.is_finite() {
        return ML_NAN;
    }
    if size < 0.0 || mu < 0.0 {
        return ML_NAN;
    }
    if size == 0.0 {
        return if x >= 0.0 { r_dt_1(lower_tail, log_p) } else { r_dt_0(lower_tail, log_p) };
    }
    if x < 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    if !x.is_finite() {
        return r_dt_1(lower_tail, log_p);
    }
    if !size.is_finite() {
        return ppois(x, mu, lower_tail, log_p);
    }
    let x = (x + 1e-7).floor();
    let (w, wc, ierr) = bratio(size, x + 1.0, size / (size + mu), mu / (size + mu), log_p);
    if ierr != 0 {
        mathlib_warning(format!("pnbinom_mu() -> bratio() gave error code {}", ierr));
    }
    if lower_tail { w } else { wc }
}

/// R: nmath/qnbinom.c:qnbinom
pub fn qnbinom(p: f64, size: f64, prob: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || size.is_nan() || prob.is_nan() {
        return p + size + prob;
    }
    if prob == 0.0 && size == 0.0 {
        return 0.0;
    }
    if prob <= 0.0 || prob > 1.0 || size < 0.0 {
        return ML_NAN;
    }
    if prob == 1.0 || size == 0.0 {
        return 0.0;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, ML_POSINF, lower_tail, log_p) {
        return v;
    }
    let q = 1.0 / prob;
    let pp = (1.0 - prob) * q;
    let mu = size * pp;
    let sigma = (size * pp * q).sqrt();
    let gamma = (q + pp) / sigma;
    discrete_quantile(p, mu, sigma, gamma, None, lower_tail, log_p, &|y, lt, lg| pnbinom(y, size, prob, lt, lg))
}

/// R: nmath/qnbinom_mu.c:qnbinom_mu
pub fn qnbinom_mu(p: f64, size: f64, mu: f64, lower_tail: bool, log_p: bool) -> f64 {
    if size == ML_POSINF {
        return qpois(p, mu, lower_tail, log_p);
    }
    if p.is_nan() || size.is_nan() || mu.is_nan() {
        return p + size + mu;
    }
    if mu == 0.0 || size == 0.0 {
        return 0.0;
    }
    if mu < 0.0 || size < 0.0 {
        return ML_NAN;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, ML_POSINF, lower_tail, log_p) {
        return v;
    }
    let q = 1.0 + mu / size;
    let pp = mu / size;
    let sigma = (size * pp * q).sqrt();
    let gamma = (q + pp) / sigma;
    discrete_quantile(p, mu, sigma, gamma, None, lower_tail, log_p, &|y, lt, lg| pnbinom_mu(y, size, mu, lt, lg))
}
