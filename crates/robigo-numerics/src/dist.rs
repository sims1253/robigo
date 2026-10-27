//! Simple distributions: uniform, exponential, Cauchy, logistic,
//! log-normal, Weibull and geometric (nmath/[dpq]{unif,exp,cauchy,logis,
//! lnorm,weibull,geom}.c).

use super::binom::dbinom_raw;
use super::norm::{pnorm, qnorm};
use super::*;

/// R: nmath/dunif.c
pub fn dunif(x: f64, a: f64, b: f64, give_log: bool) -> f64 {
    if x.is_nan() || a.is_nan() || b.is_nan() {
        return x + a + b;
    }
    if b <= a {
        return ML_NAN;
    }
    if a <= x && x <= b {
        return if give_log { -(b - a).ln() } else { 1.0 / (b - a) };
    }
    r_d__0(give_log)
}

/// R: nmath/punif.c
pub fn punif(x: f64, a: f64, b: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || a.is_nan() || b.is_nan() {
        return x + a + b;
    }
    if b < a {
        return ML_NAN;
    }
    if !a.is_finite() || !b.is_finite() {
        return ML_NAN;
    }
    if x >= b {
        return r_dt_1(lower_tail, log_p);
    }
    if x <= a {
        return r_dt_0(lower_tail, log_p);
    }
    if lower_tail { r_d_val((x - a) / (b - a), log_p) } else { r_d_val((b - x) / (b - a), log_p) }
}

/// R: nmath/qunif.c
pub fn qunif(p: f64, a: f64, b: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || a.is_nan() || b.is_nan() {
        return p + a + b;
    }
    if r_q_p01_check(p, log_p) {
        return ML_NAN;
    }
    if !a.is_finite() || !b.is_finite() {
        return ML_NAN;
    }
    if b < a {
        return ML_NAN;
    }
    if b == a {
        return a;
    }
    a + r_dt_qiv(p, lower_tail, log_p) * (b - a)
}

/// R: nmath/dexp.c
pub fn dexp(x: f64, scale: f64, give_log: bool) -> f64 {
    if x.is_nan() || scale.is_nan() {
        return x + scale;
    }
    if scale <= 0.0 {
        return ML_NAN;
    }
    if x < 0.0 {
        return r_d__0(give_log);
    }
    if give_log { (-x / scale) - scale.ln() } else { (-x / scale).exp() / scale }
}

/// R: nmath/pexp.c
pub fn pexp(x: f64, scale: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || scale.is_nan() {
        return x + scale;
    }
    if scale < 0.0 {
        return ML_NAN;
    }
    if x <= 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    let x = -(x / scale);
    if lower_tail { if log_p { r_log1_exp(x) } else { -expm1(x) } } else { r_d_exp(x, log_p) }
}

/// R: nmath/qexp.c
pub fn qexp(p: f64, scale: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || scale.is_nan() {
        return p + scale;
    }
    if scale < 0.0 {
        return ML_NAN;
    }
    if r_q_p01_check(p, log_p) {
        return ML_NAN;
    }
    if p == r_dt_0(lower_tail, log_p) {
        return 0.0;
    }
    -scale * r_dt_clog(p, lower_tail, log_p)
}

/// R: nmath/dcauchy.c
pub fn dcauchy(x: f64, location: f64, scale: f64, give_log: bool) -> f64 {
    if x.is_nan() || location.is_nan() || scale.is_nan() {
        return x + location + scale;
    }
    if scale <= 0.0 {
        return ML_NAN;
    }
    let y = (x - location) / scale;
    if give_log { -(M_PI * scale * (1.0 + y * y)).ln() } else { 1.0 / (M_PI * scale * (1.0 + y * y)) }
}

/// R: nmath/pcauchy.c (HAVE_ATANPI)
pub fn pcauchy(x: f64, location: f64, scale: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || location.is_nan() || scale.is_nan() {
        return x + location + scale;
    }
    if scale <= 0.0 {
        return ML_NAN;
    }
    let mut x = (x - location) / scale;
    if x.is_nan() {
        return ML_NAN;
    }
    if !x.is_finite() {
        return if x < 0.0 { r_dt_0(lower_tail, log_p) } else { r_dt_1(lower_tail, log_p) };
    }
    if !lower_tail {
        x = -x;
    }
    if x.abs() > 1.0 {
        let y = atanpi(1.0 / x);
        if x > 0.0 { r_d_clog(y, log_p) } else { r_d_val(-y, log_p) }
    } else {
        r_d_val(0.5 + atanpi(x), log_p)
    }
}

/// R: nmath/qcauchy.c
pub fn qcauchy(p: f64, location: f64, scale: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || location.is_nan() || scale.is_nan() {
        return p + location + scale;
    }
    if r_q_p01_check(p, log_p) {
        return ML_NAN;
    }
    if scale <= 0.0 || !scale.is_finite() {
        if scale == 0.0 {
            return location;
        }
        return ML_NAN;
    }
    let mut lower_tail = lower_tail;
    let my_inf = |lt: bool| location + (if lt { scale } else { -scale }) * ML_POSINF;
    let mut p = p;
    if log_p {
        if p > -1.0 {
            if p == 0.0 {
                return my_inf(lower_tail);
            }
            lower_tail = !lower_tail;
            p = -expm1(p);
        } else {
            p = p.exp();
        }
    } else if p > 0.5 {
        if p == 1.0 {
            return my_inf(lower_tail);
        }
        p = 1.0 - p;
        lower_tail = !lower_tail;
    }
    if p == 0.5 {
        return location;
    }
    if p == 0.0 {
        return location + (if lower_tail { scale } else { -scale }) * ML_NEGINF;
    }
    location + (if lower_tail { -scale } else { scale }) / tanpi(p)
}

/// R: nmath/dlogis.c
pub fn dlogis(x: f64, location: f64, scale: f64, give_log: bool) -> f64 {
    if x.is_nan() || location.is_nan() || scale.is_nan() {
        return x + location + scale;
    }
    if scale <= 0.0 {
        return ML_NAN;
    }
    let x = ((x - location) / scale).abs();
    let e = (-x).exp();
    let f = 1.0 + e;
    if give_log { -(x + (scale * f * f).ln()) } else { e / (scale * f * f) }
}

/// R: nmath/plogis.c:log1pexp
pub fn log1pexp(x: f64) -> f64 {
    if x <= 18.0 {
        return log1p(x.exp());
    }
    if x > 33.3 {
        return x;
    }
    x + (-x).exp()
}

/// R: nmath/plogis.c
pub fn plogis(x: f64, location: f64, scale: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || location.is_nan() || scale.is_nan() {
        return x + location + scale;
    }
    if scale <= 0.0 {
        return ML_NAN;
    }
    let x = (x - location) / scale;
    if x.is_nan() {
        return ML_NAN;
    }
    if !x.is_finite() {
        return if x > 0.0 { r_dt_1(lower_tail, log_p) } else { r_dt_0(lower_tail, log_p) };
    }
    if log_p {
        -log1pexp(if lower_tail { -x } else { x })
    } else {
        1.0 / (1.0 + (if lower_tail { -x } else { x }).exp())
    }
}

/// R: nmath/qlogis.c
pub fn qlogis(p: f64, location: f64, scale: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || location.is_nan() || scale.is_nan() {
        return p + location + scale;
    }
    if let Some(v) = r_q_p01_boundaries(p, ML_NEGINF, ML_POSINF, lower_tail, log_p) {
        return v;
    }
    if scale < 0.0 {
        return ML_NAN;
    }
    if scale == 0.0 {
        return location;
    }
    let p = if log_p {
        if lower_tail { p - r_log1_exp(p) } else { r_log1_exp(p) - p }
    } else {
        (if lower_tail { p / (1.0 - p) } else { (1.0 - p) / p }).ln()
    };
    location + scale * p
}

/// R: nmath/dlnorm.c
pub fn dlnorm(x: f64, meanlog: f64, sdlog: f64, give_log: bool) -> f64 {
    if x.is_nan() || meanlog.is_nan() || sdlog.is_nan() {
        return x + meanlog + sdlog;
    }
    if sdlog < 0.0 {
        return ML_NAN;
    }
    if !x.is_finite() && x.ln() == meanlog {
        return ML_NAN;
    }
    if sdlog == 0.0 {
        return if x.ln() == meanlog { ML_POSINF } else { r_d__0(give_log) };
    }
    if x <= 0.0 {
        return r_d__0(give_log);
    }
    let y = (x.ln() - meanlog) / sdlog;
    if give_log {
        -(M_LN_SQRT_2PI + 0.5 * y * y + (x * sdlog).ln())
    } else {
        M_1_SQRT_2PI * (-0.5 * y * y).exp() / (x * sdlog)
    }
}

/// R: nmath/plnorm.c
pub fn plnorm(x: f64, meanlog: f64, sdlog: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || meanlog.is_nan() || sdlog.is_nan() {
        return x + meanlog + sdlog;
    }
    if sdlog < 0.0 {
        return ML_NAN;
    }
    if x > 0.0 {
        return pnorm(x.ln(), meanlog, sdlog, lower_tail, log_p);
    }
    r_dt_0(lower_tail, log_p)
}

/// R: nmath/qlnorm.c
pub fn qlnorm(p: f64, meanlog: f64, sdlog: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || meanlog.is_nan() || sdlog.is_nan() {
        return p + meanlog + sdlog;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, ML_POSINF, lower_tail, log_p) {
        return v;
    }
    qnorm(p, meanlog, sdlog, lower_tail, log_p).exp()
}

/// R: nmath/dweibull.c
pub fn dweibull(x: f64, shape: f64, scale: f64, give_log: bool) -> f64 {
    if x.is_nan() || shape.is_nan() || scale.is_nan() {
        return x + shape + scale;
    }
    if shape <= 0.0 || scale <= 0.0 {
        return ML_NAN;
    }
    if x < 0.0 {
        return r_d__0(give_log);
    }
    if !x.is_finite() {
        return r_d__0(give_log);
    }
    if x == 0.0 && shape < 1.0 {
        return ML_POSINF;
    }
    let tmp1 = (x / scale).powf(shape - 1.0);
    let tmp2 = tmp1 * (x / scale);
    if give_log { -tmp2 + (shape * tmp1 / scale).ln() } else { shape * tmp1 * (-tmp2).exp() / scale }
}

/// R: nmath/pweibull.c
pub fn pweibull(x: f64, shape: f64, scale: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || shape.is_nan() || scale.is_nan() {
        return x + shape + scale;
    }
    if shape <= 0.0 || scale <= 0.0 {
        return ML_NAN;
    }
    if x <= 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    let x = -(x / scale).powf(shape);
    if lower_tail { if log_p { r_log1_exp(x) } else { -expm1(x) } } else { r_d_exp(x, log_p) }
}

/// R: nmath/qweibull.c
pub fn qweibull(p: f64, shape: f64, scale: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || shape.is_nan() || scale.is_nan() {
        return p + shape + scale;
    }
    if shape <= 0.0 || scale <= 0.0 {
        return ML_NAN;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, ML_POSINF, lower_tail, log_p) {
        return v;
    }
    scale * (-r_dt_clog(p, lower_tail, log_p)).powf(1.0 / shape)
}

/// R: nmath/dgeom.c
pub fn dgeom(x: f64, p: f64, give_log: bool) -> f64 {
    if x.is_nan() || p.is_nan() {
        return x + p;
    }
    if p <= 0.0 || p > 1.0 {
        return ML_NAN;
    }
    if let Some(v) = r_d_nonint_check(x, give_log) {
        return v;
    }
    if x < 0.0 || !x.is_finite() || p == 0.0 {
        return r_d__0(give_log);
    }
    let x = r_forceint(x);
    let prob = dbinom_raw(0.0, x, p, 1.0 - p, give_log);
    if give_log { p.ln() + prob } else { p * prob }
}

/// R: nmath/pgeom.c
pub fn pgeom(x: f64, p: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || p.is_nan() {
        return x + p;
    }
    if p <= 0.0 || p > 1.0 {
        return ML_NAN;
    }
    if x < 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    if !x.is_finite() {
        return r_dt_1(lower_tail, log_p);
    }
    let x = (x + 1e-7).floor();
    if p == 1.0 {
        let x = if lower_tail { 1.0 } else { 0.0 };
        return if log_p { f64::ln(x) } else { x };
    }
    let x = log1p(-p) * (x + 1.0);
    if log_p {
        r_dt_clog(x, lower_tail, log_p)
    } else if lower_tail {
        -expm1(x)
    } else {
        x.exp()
    }
}

/// R: nmath/qgeom.c
pub fn qgeom(p: f64, prob: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || prob.is_nan() {
        return p + prob;
    }
    if prob <= 0.0 || prob > 1.0 {
        return ML_NAN;
    }
    if r_q_p01_check(p, log_p) {
        return ML_NAN;
    }
    if prob == 1.0 {
        return 0.0;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, ML_POSINF, lower_tail, log_p) {
        return v;
    }
    fmax2(0.0, (r_dt_clog(p, lower_tail, log_p) / log1p(-prob) - 1.0 - 1e-12).ceil())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_dists() {
        // GNU R 4.5.0, sprintf("%.17g", ...)
        assert_eq!(pexp(2.0, 0.5, true, false), 0.98168436111126578);
        assert_eq!(qexp(0.5, 1.0, true, false), 0.69314718055994529);
        assert_eq!(pcauchy(3.0, 0.0, 1.0, true, false), 0.89758361765043326);
        assert_eq!(qcauchy(0.9, 0.0, 1.0, true, false), 3.0776835371752544);
        assert_eq!(plogis(1.0, 0.0, 1.0, true, false), 0.7310585786300049);
        assert_eq!(qweibull(0.5, 2.0, 1.0, true, false), 0.83255461115769769);
        assert_eq!(pgeom(3.0, 0.2, true, false), 0.59040000000000004);
    }
}
