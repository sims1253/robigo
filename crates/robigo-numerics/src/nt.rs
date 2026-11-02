//! The noncentral t distribution: dnt.c, pnt.c, qnt.c.

use super::gamma::lgammafn;
use super::ldmath::{eq, gt, le, lt};
use super::ldouble::LDouble;
use super::*;

/// R: nmath/dnt.c:dnt
pub fn dnt(x: f64, df: f64, ncp: f64, give_log: bool) -> f64 {
    if x.is_nan() || df.is_nan() {
        return x + df;
    }
    // If non-positive df then error
    if df <= 0.0 {
        return ML_NAN;
    }
    if ncp == 0.0 {
        return super::tdist::dt(x, df, give_log);
    }
    // If x is infinite then return 0
    if !x.is_finite() {
        return r_d__0(give_log);
    }
    // If infinite df then the density is identical to a normal distribution
    // with mean = ncp. However, the formula loses a lot of accuracy around
    // df=1e9
    if !df.is_finite() || df > 1e8 {
        return super::norm::dnorm(x, ncp, 1.0, give_log);
    }

    // Do calculations on log scale to stabilize
    // Consider two cases: x ~= 0 or not
    let u = if x.abs() > (df * DBL_EPSILON).sqrt() {
        df.ln() - x.abs().ln()
            + (pnt(x * ((df + 2.0) / df).sqrt(), df + 2.0, ncp, true, false) - pnt(x, df, ncp, true, false)).abs().ln()
    } else {
        // x ~= 0 : -> same value as for x = 0
        lgammafn((df + 1.0) / 2.0) - lgammafn(df / 2.0) - (M_LN_SQRT_PI + 0.5 * (df.ln() + ncp * ncp))
    };

    if give_log { u } else { u.exp() }
}

/// R: nmath/pnt.c:pnt
pub fn pnt(t: f64, df: f64, ncp: f64, lower_tail: bool, log_p: bool) -> f64 {
    // note - itrmax and errmax may be changed to suit one's needs.
    const ITRMAX: i32 = 1000;
    const ERRMAX: f64 = 1e-12;

    if df <= 0.0 {
        return ML_NAN;
    }
    if ncp == 0.0 {
        return super::tdist::pt(t, df, lower_tail, log_p);
    }
    if !t.is_finite() {
        return if t < 0.0 { r_dt_0(lower_tail, log_p) } else { r_dt_1(lower_tail, log_p) };
    }

    let (negdel, tt, del) = if t >= 0.0 {
        (false, t, ncp)
    } else {
        // We deal quickly with left tail if extreme, since
        // pt(q, df, ncp) <= pt(0, df, ncp) = \Phi(-ncp)
        if ncp > 40.0 && (!log_p || !lower_tail) {
            return r_dt_0(lower_tail, log_p);
        }
        (true, -t, -ncp)
    };

    if df > 4e5 || del * del > 2.0 * M_LN2 * (-(DBL_MIN_EXP as f64)) {
        // 2nd part: if del > 37.62, then p=0 below
        // Approx. from Abramowitz & Stegun 26.7.10 (p.949)
        // R: `s` is LDOUBLE, so `1 - s` and the products with it are
        // evaluated in long double before the cast to double.
        let s = LDouble::from_f64(1.0 / (4.0 * df));
        let m = (LDouble::from_f64(tt) * (LDouble::from_f64(1.0) - s)).to_f64();
        let sd = (LDouble::from_f64(1.0) + LDouble::from_f64(tt * tt * 2.0) * s).to_f64().sqrt();
        return super::norm::pnorm(m, del, sd, lower_tail != negdel, log_p);
    }

    // initialize twin series
    // Guenther, J. (1978). Statist. Computn. Simuln. vol.6, 199.
    let mut x = t * t;
    let mut rxb = df / (x + df); // := (1 - x) {x below} -- but more accurately
    x = x / (x + df); // in [0,1)

    let tnc = if x > 0.0 {
        // <==>  t != 0
        let lambda = del * del;
        let mut p = LDouble::from_f64(0.5 * (-0.5 * lambda).exp());
        if eq(p, LDouble::from_f64(0.0)) {
            // underflow!
            // ========== really use another algorithm for this case !!!
            ml_warning(ME_UNDERFLOW, "pnt");
            ml_warning(ME_RANGE, "pnt"); // |ncp| too large
            return r_dt_0(lower_tail, log_p);
        }
        let mut q = LDouble::from_f64(M_SQRT_2DPI) * p * del;
        let mut s = LDouble::from_f64(0.5) - p;
        // s = 0.5 - p = 0.5*(1 - exp(-.5 L)) = -0.5*expm1(-.5 L))
        if lt(s, LDouble::from_f64(1e-7)) {
            s = LDouble::from_f64(-0.5 * expm1(-0.5 * lambda));
        }
        let mut a = 0.5;
        let b = 0.5 * df;
        // rxb = (1 - x) ^ b   [ ~= 1 - b*x for tiny x --> see 'xeven' below]
        //       where '(1 - x)' =: rxb {accurately!} above
        rxb = rxb.powf(b);
        let albeta = M_LN_SQRT_PI + lgammafn(b) - lgammafn(0.5 + b);
        let mut xodd = LDouble::from_f64(super::beta::pbeta(x, a, b, true, false));
        let mut godd = LDouble::from_f64(2.0 * rxb * (a * x.ln() - albeta).exp());
        let mut xeven;
        let mut geven;
        let tnc0 = LDouble::from_f64(b * x);
        if lt(tnc0, LDouble::from_f64(DBL_EPSILON)) {
            xeven = tnc0;
        } else {
            xeven = LDouble::from_f64(1.0 - rxb);
        }
        geven = tnc0 * rxb;
        let mut tnc = p * xodd + q * xeven;

        // repeat until convergence or iteration limit
        let mut it = 1;
        loop {
            a += 1.0;
            xodd -= godd;
            xeven -= geven;
            godd *= x * (a + b - 1.0) / a;
            geven *= x * (a + b - 0.5) / (a + 0.5);
            p *= lambda / (2.0 * it as f64);
            q *= lambda / (2.0 * it as f64 + 1.0);
            tnc = tnc + p * xodd + q * xeven;
            s -= p;
            // R 2.4.0 added test for rounding error here.
            if lt(s, LDouble::from_f64(-1e-10)) {
                // happens e.g. for (t,df,ncp)=(40,10,38.5), after 799 it.
                ml_warning(ME_PRECISION, "pnt");
                break;
            }
            if le(s, LDouble::from_f64(0.0)) && it > 1 {
                break;
            }
            let errbd = (LDouble::from_f64(2.0) * s * (xodd - godd)).to_f64();
            if errbd.abs() < ERRMAX {
                break; // convergence
            }
            if it == ITRMAX {
                // non-convergence:
                ml_warning(ME_NOCONV, "pnt");
                break;
            }
            it += 1;
        }
        tnc
    } else {
        // x = t = 0
        LDouble::from_f64(0.0)
    };

    let tnc = tnc + LDouble::from_f64(super::norm::pnorm(-del, 0.0, 1.0, true, false));

    let lower_tail = lower_tail != negdel; // xor
    if gt(tnc, LDouble::from_f64(1.0 - 1e-10)) && lower_tail {
        ml_warning(ME_PRECISION, "pnt{final}");
    }

    let tnc = fmin2(tnc.to_f64(), 1.0); // Precaution
    r_dt_val(tnc, lower_tail, log_p)
}

/// R: nmath/qnt.c:qnt
pub fn qnt(p: f64, df: f64, ncp: f64, lower_tail: bool, log_p: bool) -> f64 {
    const ACCU: f64 = 1e-13;
    const EPS: f64 = 1e-11; // must be > accu

    if p.is_nan() || df.is_nan() || ncp.is_nan() {
        return p + df + ncp;
    }
    // Was
    // df = floor(df + 0.5);
    // if (df < 1 || ncp < 0) ML_WARN_return_NAN;
    if df <= 0.0 {
        return ML_NAN;
    }
    if ncp == 0.0 && df >= 1.0 {
        return super::tdist::qt(p, df, lower_tail, log_p);
    }
    if let Some(v) = r_q_p01_boundaries(p, ML_NEGINF, ML_POSINF, lower_tail, log_p) {
        return v;
    }

    if !df.is_finite() {
        // df = Inf ==> limit N(ncp,1)
        return super::norm::qnorm(p, ncp, 1.0, lower_tail, log_p);
    }

    let p = r_dt_qiv(p, lower_tail, log_p);

    // Invert pnt(.):
    // 1. finding an upper and lower bound
    if p > 1.0 - DBL_EPSILON {
        return ML_POSINF;
    }
    let mut pp = fmin2(1.0 - DBL_EPSILON, p * (1.0 + EPS));
    let mut ux = fmax2(1.0, ncp);
    while ux < DBL_MAX && pnt(ux, df, ncp, true, false) < pp {
        ux *= 2.0;
    }
    pp = p * (1.0 - EPS);
    let mut lx = fmin2(-1.0, -ncp);
    while lx > -DBL_MAX && pnt(lx, df, ncp, true, false) > pp {
        lx *= 2.0;
    }

    // 2. interval (lx,ux)  halving:
    loop {
        let nx = 0.5 * (lx + ux); // could be zero
        if pnt(nx, df, ncp, true, false) > p {
            ux = nx;
        } else {
            lx = nx;
        }
        if !((ux - lx) > ACCU * fmax2(lx.abs(), ux.abs())) {
            break;
        }
    }

    0.5 * (lx + ux)
}
