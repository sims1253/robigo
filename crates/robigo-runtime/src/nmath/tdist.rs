//! Student's t distribution: dt.c, pt.c, qt.c.

#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::beta::pbeta;
use super::binom::bd0;
use super::gamma::{lbeta, stirlerr};
use super::norm::{dnorm, pnorm, qnorm};
use super::*;

/// R: nmath/dt.c:dt
pub fn dt(x: f64, n: f64, give_log: bool) -> f64 {
    if x.is_nan() || n.is_nan() {
        return x + n;
    }
    if n <= 0.0 {
        return ML_NAN;
    }
    if !x.is_finite() {
        return r_d__0(give_log);
    }
    if !n.is_finite() {
        return dnorm(x, 0.0, 1.0, give_log);
    }
    let t = -bd0(n / 2.0, (n + 1.0) / 2.0) + stirlerr((n + 1.0) / 2.0) - stirlerr(n / 2.0);
    let x2n = x * x / n;
    let mut ax = 0.0;
    let l_x2n;
    let u;
    let lrg_x2n = x2n > 1.0 / DBL_EPSILON;
    if lrg_x2n {
        ax = x.abs();
        l_x2n = ax.ln() - n.ln() / 2.0;
        u = n * l_x2n;
    } else if x2n > 0.2 {
        l_x2n = (1.0 + x2n).ln() / 2.0;
        u = n * l_x2n;
    } else {
        l_x2n = log1p(x2n) / 2.0;
        u = -bd0(n / 2.0, (n + x * x) / 2.0) + x * x / 2.0;
    }
    if give_log {
        return t - u - (M_LN_SQRT_2PI + l_x2n);
    }
    let i_sqrt_ = if lrg_x2n { n.sqrt() / ax } else { (-l_x2n).exp() };
    (t - u).exp() * M_1_SQRT_2PI * i_sqrt_
}

/// R: nmath/pt.c:pt
pub fn pt(x: f64, n: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || n.is_nan() {
        return x + n;
    }
    if n <= 0.0 {
        return ML_NAN;
    }
    if !x.is_finite() {
        return if x < 0.0 { r_dt_0(lower_tail, log_p) } else { r_dt_1(lower_tail, log_p) };
    }
    if !n.is_finite() {
        return pnorm(x, 0.0, 1.0, lower_tail, log_p);
    }
    let nx = 1.0 + (x / n) * x;
    let mut val;
    if nx > 1e100 {
        let lval = -0.5 * n * (2.0 * x.abs().ln() - n.ln()) - lbeta(0.5 * n, 0.5) - (0.5 * n).ln();
        val = if log_p { lval } else { lval.exp() };
    } else {
        val = if n > x * x {
            pbeta(x * x / (n + x * x), 0.5, n / 2.0, false, log_p)
        } else {
            pbeta(1.0 / nx, n / 2.0, 0.5, true, log_p)
        };
    }
    let mut lower_tail = lower_tail;
    if x <= 0.0 {
        lower_tail = !lower_tail;
    }
    if log_p {
        if lower_tail { log1p(-0.5 * val.exp()) } else { val - M_LN2 }
    } else {
        val /= 2.0;
        r_d_cval(val, lower_tail)
    }
}

/// R: nmath/qt.c:qt
pub fn qt(p: f64, ndf: f64, lower_tail: bool, log_p: bool) -> f64 {
    const EPS: f64 = 1.0e-12;
    if p.is_nan() || ndf.is_nan() {
        return p + ndf;
    }
    if let Some(v) = r_q_p01_boundaries(p, ML_NEGINF, ML_POSINF, lower_tail, log_p) {
        return v;
    }
    if ndf <= 0.0 {
        return ML_NAN;
    }
    let mut p = p;
    if ndf < 1.0 {
        const ACCU: f64 = 1e-13;
        const EPS_: f64 = 1e-11;
        let mut iter = 0;
        p = r_dt_qiv(p, lower_tail, log_p);
        if p > 1.0 - DBL_EPSILON {
            return ML_POSINF;
        }
        let mut pp = fmin2(1.0 - DBL_EPSILON, p * (1.0 + EPS_));
        let mut ux = 1.0;
        while ux < DBL_MAX && pt(ux, ndf, true, false) < pp {
            ux *= 2.0;
        }
        pp = p * (1.0 - EPS_);
        let mut lx = -1.0;
        while lx > -DBL_MAX && pt(lx, ndf, true, false) > pp {
            lx *= 2.0;
        }
        loop {
            let nx = 0.5 * (lx + ux);
            if pt(nx, ndf, true, false) > p {
                ux = nx;
            } else {
                lx = nx;
            }
            // C: while ((ux - lx) / fabs(nx) > accu && ++iter < 1000)
            if !((ux - lx) / nx.abs() > ACCU && {
                iter += 1;
                iter < 1000
            }) {
                break;
            }
        }
        if iter >= 1000 {
            ml_warning(ME_PRECISION, "qt");
        }
        return 0.5 * (lx + ux);
    }

    if ndf > 1e20 {
        return qnorm(p, 0.0, 1.0, lower_tail, log_p);
    }

    let mut pp_ = r_d_qiv(p, log_p);
    let neg = (!lower_tail || pp_ < 0.5) && (lower_tail || pp_ > 0.5);
    let is_neg_lower = lower_tail == neg;
    if neg {
        pp_ = 2.0 * if log_p { if lower_tail { pp_ } else { -expm1(p) } } else { r_d_lval(p, lower_tail) };
    } else {
        pp_ = 2.0 * if log_p { if lower_tail { -expm1(p) } else { pp_ } } else { r_d_cval(p, lower_tail) };
    }
    let big_p = pp_;
    let mut q;

    if (ndf - 2.0).abs() < EPS {
        if big_p > DBL_MIN {
            if 3.0 * big_p < DBL_EPSILON {
                q = 1.0 / big_p.sqrt();
            } else if big_p > 0.9 {
                q = (1.0 - big_p) * (2.0 / (big_p * (2.0 - big_p))).sqrt();
            } else {
                q = (2.0 / (big_p * (2.0 - big_p)) - 2.0).sqrt();
            }
        } else if log_p {
            q = if is_neg_lower { (-p / 2.0).exp() / M_SQRT2 } else { 1.0 / (-expm1(p)).sqrt() };
        } else {
            q = ML_POSINF;
        }
    } else if ndf < 1.0 + EPS {
        if big_p == 1.0 {
            q = 0.0;
        } else if big_p > 0.0 {
            q = 1.0 / tanpi(big_p / 2.0);
        } else if log_p {
            q = if is_neg_lower { M_1_PI * (-p).exp() } else { -1.0 / (M_PI * expm1(p)) };
        } else {
            q = ML_POSINF;
        }
    } else {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut log_p2 = 0.0;
        let a = 1.0 / (ndf - 0.5);
        let b = 48.0 / (a * a);
        let mut c = ((20700.0 * a / b - 98.0) * a - 16.0) * a + 96.36;
        let d = ((94.5 / (b + c) - 3.0) / b + 1.0) * (a * M_PI_2).sqrt() * ndf;

        let p_ok1 = big_p > DBL_MIN || !log_p;
        let mut p_ok = p_ok1;
        if p_ok1 {
            y = (d * big_p).powf(2.0 / ndf);
            p_ok = y >= DBL_EPSILON;
        }
        if !p_ok {
            log_p2 = if is_neg_lower { r_d_log(p, log_p) } else { r_d_lexp(p, log_p) };
            x = (d.ln() + M_LN2 + log_p2) / ndf;
            y = (2.0 * x).exp();
        }
        if (ndf < 2.1 && big_p > 0.5) || y > 0.05 + a {
            if p_ok {
                x = qnorm(0.5 * big_p, 0.0, 1.0, true, false);
            } else {
                x = qnorm(log_p2, 0.0, 1.0, lower_tail, true);
            }
            y = x * x;
            if ndf < 5.0 {
                c += 0.3 * (ndf - 4.5) * (x + 0.6);
            }
            c = (((0.05 * d * x - 5.0) * x - 7.0) * x - 2.0) * x + b + c;
            y = (((((0.4 * y + 6.3) * y + 36.0) * y + 94.5) / c - y - 3.0) / b + 1.0) * x;
            y = expm1(a * y * y);
            q = (ndf * y).sqrt();
        } else if !p_ok && x < -M_LN2 * DBL_MANT_DIG as f64 {
            q = ndf.sqrt() * (-x).exp();
        } else {
            y = ((1.0 / (((ndf + 6.0) / (ndf * y) - 0.089 * d - 0.822) * (ndf + 2.0) * 3.0) + 0.5 / (ndf + 4.0))
                * y
                - 1.0)
                * (ndf + 1.0)
                / (ndf + 2.0)
                + 1.0 / y;
            q = (ndf * y).sqrt();
        }

        if p_ok1 {
            let m = ((DBL_MAX / 2.0).sqrt() - ndf).abs();
            let mut it = 0;
            loop {
                let cont = it < 10;
                it += 1;
                if !cont {
                    break;
                }
                y = dt(q, ndf, false);
                if !(y > 0.0) {
                    break;
                }
                x = (pt(q, ndf, false, false) - big_p / 2.0) / y;
                if !x.is_finite() {
                    break;
                }
                if !(x.abs() > 1e-14 * q.abs()) {
                    break;
                }
                let f = if q.abs() < m {
                    q * (ndf + 1.0) / (2.0 * (q * q + ndf))
                } else {
                    (ndf + 1.0) / (2.0 * (q + ndf / q))
                };
                let del_q = x * (1.0 + x * f);
                if del_q.is_finite() && (q + del_q).is_finite() {
                    q += del_q;
                } else if x.is_finite() && (q + x).is_finite() {
                    q += x;
                } else {
                    break;
                }
            }
        }
    }
    if neg { -q } else { q }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collects mismatches so that one run reports every failing value.
    macro_rules! chk {
        ($bad:ident, $e:expr, $v:expr) => {{
            let got: f64 = $e;
            let want: f64 = $v;
            if !(got == want || (got.is_nan() && want.is_nan())) {
                $bad.push(format!("{} = {:?} (want {:?})", stringify!($e), got, want));
            }
        }};
    }

    // Reference values: GNU R 4.5.0, sprintf("%.17g", .Internal(dt(...))) etc.
    #[test]
    fn dt_values() {
        let mut bad: Vec<String> = Vec::new();
        chk!(bad, dt(1.5, 3.0, false), 0.12001717451358741);
        chk!(bad, dt(1.5, 3.0, true), -2.1201204254943549);
        chk!(bad, dt(0.2, 1e+06, false), 0.39104258855035995);
        chk!(bad, dt(100.0, 2.5, false), 1.7975627902605629e-07);
        chk!(bad, dt(100.0, 2.5, true), -15.531663908960613);
        chk!(bad, dt(1e+200, 3.0, false), 0.0);
        chk!(bad, dt(1e+200, 3.0, true), -1840.8717386675239);
        chk!(bad, dt(2.0, f64::INFINITY, false), 0.053990966513188063);
        chk!(bad, dt(0.0, 0.5, false), 0.26967630059418968);
        chk!(bad, dt(-3.0, 0.01, true), -6.4383930752660534);
        chk!(bad, dt(0.3, -1.0, false), f64::NAN);
        assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
    }

    #[test]
    fn pt_values() {
        let mut bad: Vec<String> = Vec::new();
        chk!(bad, pt(1.96, 10.0, true, false), 0.96078187987615016);
        chk!(bad, pt(-3.0, 4.0, true, false), 0.019970984035859413);
        chk!(bad, pt(-3.0, 4.0, true, true), -3.9134748570618902);
        chk!(bad, pt(-3.0, 4.0, false, false), 0.9800290159641406);
        chk!(bad, pt(3.0, 4.0, false, true), -3.9134748570618902);
        chk!(bad, pt(1e+60, 3.0, false, false), 1.1026577908435548e-180);
        chk!(bad, pt(-1e+60, 3.0, true, true), -414.36759329988365);
        chk!(bad, pt(2.0, f64::INFINITY, true, false), 0.97724986805182079);
        chk!(bad, pt(0.5, 0.3, true, false), 0.5996446036892612);
        chk!(bad, pt(40.0, 30.0, false, false), 6.8630225972031808e-28);
        chk!(bad, pt(40.0, 30.0, false, true), -62.54623464729918);
        chk!(bad, pt(0.1, 1e+10, true, false), 0.53982783727602657);
        chk!(bad, pt(f64::INFINITY, 3.0, true, false), 1.0);
        assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
    }

    #[test]
    fn qt_values() {
        let mut bad: Vec<String> = Vec::new();
        chk!(bad, qt(0.975, 10.0, true, false), 2.2281388519862739);
        chk!(bad, qt(0.975, 1.0, true, false), 12.706204736174694);
        chk!(bad, qt(0.975, 2.0, true, false), 4.3026527297494619);
        chk!(bad, qt(0.3, 0.5, true, false), -1.0095258786071923);
        chk!(bad, qt(1e-10, 3.0, true, false), -2225.7692846830946);
        chk!(bad, qt(-100.0, 5.0, true, true), -760929482.41469133);
        chk!(bad, qt(-100.0, 5.0, false, true), 760929482.41469133);
        chk!(bad, qt(0.6, 1.5, true, false), 0.30074539256979543);
        chk!(bad, qt(0.99, 1e+25, true, false), 2.3263478740408408);
        chk!(bad, qt(0.1, 30.0, false, false), 1.3104150253913953);
        chk!(bad, qt(1e-300, 2.0, true, false), -7.0710678118654748e+149);
        chk!(bad, qt(-800.0, 2.0, true, true), -3.6921366253922445e+173);
        chk!(bad, qt(-800.0, 1.0, true, true), f64::NEG_INFINITY);
        chk!(bad, qt(0.9, 1.05, true, false), 2.9256972731648911);
        chk!(bad, qt(1e-20, 1.3, true, false), -1073533092865201.8);
        chk!(bad, qt(-2.0, 1.01, true, true), -2.1905782829509954);
        chk!(bad, qt(0.5, 7.0, true, false), 0.0);
        assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
    }
}
