//! The normal distribution: dnorm.c, pnorm.c, qnorm.c.

use super::*;

/// R: nmath/dnorm.c:dnorm4
pub fn dnorm(x: f64, mu: f64, sigma: f64, give_log: bool) -> f64 {
    if x.is_nan() || mu.is_nan() || sigma.is_nan() {
        return x + mu + sigma;
    }
    if sigma < 0.0 {
        return ML_NAN;
    }
    if !sigma.is_finite() {
        return r_d__0(give_log);
    }
    if !x.is_finite() && mu == x {
        return ML_NAN;
    }
    if sigma == 0.0 {
        return if x == mu { ML_POSINF } else { r_d__0(give_log) };
    }
    let x = (x - mu) / sigma;
    if !x.is_finite() {
        return r_d__0(give_log);
    }
    let x = x.abs();
    if x >= 2.0 * DBL_MAX.sqrt() {
        return r_d__0(give_log);
    }
    if give_log {
        return -(M_LN_SQRT_2PI + 0.5 * x * x + sigma.ln());
    }
    if x < 5.0 {
        return M_1_SQRT_2PI * (-0.5 * x * x).exp() / sigma;
    }
    if x > (-2.0 * M_LN2 * (DBL_MIN_EXP + 1 - DBL_MANT_DIG) as f64).sqrt() {
        return 0.0;
    }
    let x1 = ldexp(r_forceint(ldexp(x, 16)), -16);
    let x2 = x - x1;
    M_1_SQRT_2PI / sigma * ((-0.5 * x1 * x1).exp() * ((-0.5 * x2 - x1) * x2).exp())
}

/// R: nmath/pnorm.c:pnorm5
pub fn pnorm(x: f64, mu: f64, sigma: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || mu.is_nan() || sigma.is_nan() {
        return x + mu + sigma;
    }
    if !x.is_finite() && mu == x {
        return ML_NAN;
    }
    if sigma <= 0.0 {
        if sigma < 0.0 {
            return ML_NAN;
        }
        return if x < mu { r_dt_0(lower_tail, log_p) } else { r_dt_1(lower_tail, log_p) };
    }
    let p = (x - mu) / sigma;
    if !p.is_finite() {
        return if x < mu { r_dt_0(lower_tail, log_p) } else { r_dt_1(lower_tail, log_p) };
    }
    let (cum, ccum) = pnorm_both(p, if lower_tail { 0 } else { 1 }, log_p);
    if lower_tail {
        cum
    } else {
        ccum
    }
}

/// R: nmath/pnorm.c:pnorm_both. Returns (cum, ccum); `i_tail` in
/// {0, 1, 2} = lower, upper, both. Unrequested parts are unspecified.
pub fn pnorm_both(x: f64, i_tail: i32, log_p: bool) -> (f64, f64) {
    const A: [f64; 5] = [
        2.2352520354606839287,
        161.02823106855587881,
        1067.6894854603709582,
        18154.981253343561249,
        0.065682337918207449113,
    ];
    const B: [f64; 4] = [47.20258190468824187, 976.09855173777669322, 10260.932208618978205, 45507.789335026729956];
    const C: [f64; 9] = [
        0.39894151208813466764,
        8.8831497943883759412,
        93.506656132177855979,
        597.27027639480026226,
        2494.5375852903726711,
        6848.1904505362823326,
        11602.651437647350124,
        9842.7148383839780218,
        1.0765576773720192317e-8,
    ];
    const D: [f64; 8] = [
        22.266688044328115691,
        235.38790178262499861,
        1519.377599407554805,
        6485.558298266760755,
        18615.571640885098091,
        34900.952721145977266,
        38912.003286093271411,
        19685.429676859990727,
    ];
    const P: [f64; 6] = [
        0.21589853405795699,
        0.1274011611602473639,
        0.022235277870649807,
        0.001421619193227893466,
        2.9112874951168792e-5,
        0.02307344176494017303,
    ];
    const Q: [f64; 5] = [
        1.28426009614491121,
        0.468238212480865118,
        0.0659881378689285515,
        0.00378239633202758244,
        7.29751555083966205e-5,
    ];

    if x.is_nan() {
        return (x, x);
    }
    let mut cum = 0.0;
    let mut ccum = 0.0;
    let eps = DBL_EPSILON * 0.5;
    let lower = i_tail != 1;
    let upper = i_tail != 0;

    let d_2 = |v: f64| ldexp(v, -1);
    // do_del(X) followed by swap_tail
    let do_del = |xv: f64, temp: f64, cum: &mut f64, ccum: &mut f64| {
        let xsq = ldexp(ldexp(xv, 4).trunc(), -4);
        let del = (xv - xsq) * (xv + xsq);
        if log_p {
            *cum = (-xsq * d_2(xsq)) - d_2(del) + temp.ln();
            if (lower && x > 0.0) || (upper && x <= 0.0) {
                *ccum = log1p(-(-xsq * d_2(xsq)).exp() * (-d_2(del)).exp() * temp);
            }
        } else {
            *cum = (-xsq * d_2(xsq)).exp() * (-d_2(del)).exp() * temp;
            *ccum = 1.0 - *cum;
        }
        if x > 0.0 {
            let t = *cum;
            if lower {
                *cum = *ccum;
            }
            *ccum = t;
        }
    };

    let y = x.abs();
    if y <= 0.67448975 {
        let (xnum, xden);
        if y > eps {
            let xsq = x * x;
            let mut n = A[4] * xsq;
            let mut d = xsq;
            for i in 0..3 {
                n = (n + A[i]) * xsq;
                d = (d + B[i]) * xsq;
            }
            xnum = n;
            xden = d;
        } else {
            xnum = 0.0;
            xden = 0.0;
        }
        let temp = x * (xnum + A[3]) / (xden + B[3]);
        if lower {
            cum = 0.5 + temp;
        }
        if upper {
            ccum = 0.5 - temp;
        }
        if log_p {
            if lower {
                cum = cum.ln();
            }
            if upper {
                ccum = ccum.ln();
            }
        }
    } else if y <= M_SQRT_32 {
        let mut xnum = C[8] * y;
        let mut xden = y;
        for i in 0..7 {
            xnum = (xnum + C[i]) * y;
            xden = (xden + D[i]) * y;
        }
        let temp = (xnum + C[7]) / (xden + D[7]);
        do_del(y, temp, &mut cum, &mut ccum);
    } else if (log_p && y < 1e170)
        || (lower && -38.4674 < x && x < 8.2924)
        || (upper && -8.2924 < x && x < 38.4674)
    {
        let xsq = 1.0 / (x * x);
        let mut xnum = P[5] * xsq;
        let mut xden = xsq;
        for i in 0..4 {
            xnum = (xnum + P[i]) * xsq;
            xden = (xden + Q[i]) * xsq;
        }
        let mut temp = xsq * (xnum + P[4]) / (xden + Q[4]);
        temp = (M_1_SQRT_2PI - temp) / y;
        do_del(x, temp, &mut cum, &mut ccum);
    } else if x > 0.0 {
        cum = r_d__1(log_p);
        ccum = r_d__0(log_p);
    } else {
        cum = r_d__0(log_p);
        ccum = r_d__1(log_p);
    }
    (cum, ccum)
}

/// R: nmath/qnorm.c:qnorm5
pub fn qnorm(p: f64, mu: f64, sigma: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || mu.is_nan() || sigma.is_nan() {
        return p + mu + sigma;
    }
    if let Some(v) = r_q_p01_boundaries(p, ML_NEGINF, ML_POSINF, lower_tail, log_p) {
        return v;
    }
    if sigma < 0.0 {
        return ML_NAN;
    }
    if sigma == 0.0 {
        return mu;
    }
    let p_ = r_dt_qiv(p, lower_tail, log_p);
    let q = p_ - 0.5;
    let mut r;
    let mut val;
    if q.abs() <= 0.425 {
        r = 0.180625 - q * q;
        val = q
            * (((((((r * 2509.0809287301226727 + 33430.575583588128105) * r + 67265.770927008700853) * r
                + 45921.953931549871457)
                * r
                + 13731.693765509461125)
                * r
                + 1971.5909503065514427)
                * r
                + 133.14166789178437745)
                * r
                + 3.387132872796366608)
            / (((((((r * 5226.495278852854561 + 28729.085735721942674) * r + 39307.89580009271061) * r
                + 21213.794301586595867)
                * r
                + 5394.1960214247511077)
                * r
                + 687.1870074920579083)
                * r
                + 42.313330701600911252)
                * r
                + 1.0);
    } else {
        let lp = if log_p && ((lower_tail && q <= 0.0) || (!lower_tail && q > 0.0)) {
            p
        } else {
            (if q > 0.0 { r_dt_civ(p, lower_tail, log_p) } else { p_ }).ln()
        };
        r = (-lp).sqrt();
        if r <= 5.0 {
            r += -1.6;
            val = (((((((r * 7.7454501427834140764e-4 + 0.0227238449892691845833) * r + 0.24178072517745061177)
                * r
                + 1.27045825245236838258)
                * r
                + 3.64784832476320460504)
                * r
                + 5.7694972214606914055)
                * r
                + 4.6303378461565452959)
                * r
                + 1.42343711074968357734)
                / (((((((r * 1.05075007164441684324e-9 + 5.475938084995344946e-4) * r + 0.0151986665636164571966)
                    * r
                    + 0.14810397642748007459)
                    * r
                    + 0.68976733498510000455)
                    * r
                    + 1.6763848301838038494)
                    * r
                    + 2.05319162663775882187)
                    * r
                    + 1.0);
        } else if r <= 27.0 {
            r += -5.0;
            val = (((((((r * 2.01033439929228813265e-7 + 2.71155556874348757815e-5) * r + 0.0012426609473880784386)
                * r
                + 0.026532189526576123093)
                * r
                + 0.29656057182850489123)
                * r
                + 1.7848265399172913358)
                * r
                + 5.4637849111641143699)
                * r
                + 6.6579046435011037772)
                / (((((((r * 2.04426310338993978564e-15 + 1.4215117583164458887e-7) * r + 1.8463183175100546818e-5)
                    * r
                    + 7.868691311456132591e-4)
                    * r
                    + 0.0148753612908506148525)
                    * r
                    + 0.13692988092273580531)
                    * r
                    + 0.59983220655588793769)
                    * r
                    + 1.0);
        } else if r >= 6.4e8 {
            val = r * M_SQRT2;
        } else {
            let s2 = -ldexp(lp, 1);
            let mut x2 = s2 - (M_2PI * s2).ln();
            if r < 36000.0 {
                x2 = s2 - (M_2PI * x2).ln() - 2.0 / (2.0 + x2);
                if r < 840.0 {
                    x2 = s2 - (M_2PI * x2).ln() + 2.0 * log1p(-(1.0 - 1.0 / (4.0 + x2)) / (2.0 + x2));
                    if r < 109.0 {
                        x2 = s2 - (M_2PI * x2).ln()
                            + 2.0 * log1p(-(1.0 - (1.0 - 5.0 / (6.0 + x2)) / (4.0 + x2)) / (2.0 + x2));
                        if r < 55.0 {
                            x2 = s2 - (M_2PI * x2).ln()
                                + 2.0
                                    * log1p(
                                        -(1.0 - (1.0 - (5.0 - 9.0 / (8.0 + x2)) / (6.0 + x2)) / (4.0 + x2)) / (2.0 + x2),
                                    );
                        }
                    }
                }
            }
            val = x2.sqrt();
        }
        if q < 0.0 {
            val = -val;
        }
    }
    mu + sigma * val
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_values() {
        // GNU R 4.5.0, sprintf("%.17g", ...)
        assert_eq!(dnorm(1.0, 0.0, 1.0, false), 0.24197072451914337);
        assert_eq!(dnorm(10.0, 0.0, 1.0, false), 7.6945986267064199e-23);
        assert_eq!(pnorm(1.96, 0.0, 1.0, true, false), 0.97500210485177963);
        assert_eq!(pnorm(-10.0, 0.0, 1.0, true, false), 7.6198530241605269e-24);
        assert_eq!(pnorm(3.0, 0.0, 1.0, false, true), -6.6077262215103492);
        assert_eq!(qnorm(0.975, 0.0, 1.0, true, false), 1.9599639845400536);
        assert_eq!(qnorm(1e-300, 0.0, 1.0, true, false), -37.047096299361201);
        assert_eq!(qnorm(-1e5, 0.0, 1.0, true, true), -447.19789367852508);
    }
}
