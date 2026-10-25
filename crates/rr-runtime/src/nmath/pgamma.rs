//! The gamma and Poisson distributions: pgamma.c (Morten Welinder's
//! algorithm), dgamma.c, qgamma.c, dpois.c, ppois.c, qpois.c and the
//! chi-squared wrappers (dchisq.c, pchisq.c, qchisq.c).

use super::binom::{discrete_quantile, ebd0};
use super::gamma::{lgammafn, stirlerr};
use super::norm::{dnorm, pnorm, qnorm};
use super::*;

/// (2^32)^8 = 2^256
const SCALEFACTOR: f64 = 1.157920892373162e77;
/// M_LN2 * DBL_MAX_EXP / DBL_EPSILON
const M_CUTOFF: f64 = M_LN2 * DBL_MAX_EXP as f64 / DBL_EPSILON;

/// R: nmath/pgamma.c:logcf
fn logcf(x: f64, i: f64, d: f64, eps: f64) -> f64 {
    let mut c1 = 2.0 * d;
    let mut c2 = i + d;
    let mut c4 = c2 + d;
    let mut a1 = c2;
    let mut b1 = i * (c2 - i * x);
    let mut b2 = d * d * x;
    let mut a2 = c4 * c2 - b2;
    b2 = c4 * b1 - i * b2;

    while (a2 * b1 - a1 * b2).abs() > (eps * b1 * b2).abs() {
        let mut c3 = c2 * c2 * x;
        c2 += d;
        c4 += d;
        a1 = c4 * a2 - c3 * a1;
        b1 = c4 * b2 - c3 * b1;

        c3 = c1 * c1 * x;
        c1 += d;
        c4 += d;
        a2 = c4 * a1 - c3 * a2;
        b2 = c4 * b1 - c3 * b2;

        if b2.abs() > SCALEFACTOR {
            a1 /= SCALEFACTOR;
            b1 /= SCALEFACTOR;
            a2 /= SCALEFACTOR;
            b2 /= SCALEFACTOR;
        } else if b2.abs() < 1.0 / SCALEFACTOR {
            a1 *= SCALEFACTOR;
            b1 *= SCALEFACTOR;
            a2 *= SCALEFACTOR;
            b2 *= SCALEFACTOR;
        }
    }
    a2 / b2
}

/// R: nmath/pgamma.c:log1pmx -- log(1+x) - x
pub fn log1pmx(x: f64) -> f64 {
    const MIN_LOG1_VALUE: f64 = -0.79149064;
    if !(MIN_LOG1_VALUE..=1.0).contains(&x) {
        log1p(x) - x
    } else {
        let r = x / (2.0 + x);
        let y = r * r;
        if x.abs() < 1e-2 {
            const TWO: f64 = 2.0;
            r * ((((TWO / 9.0 * y + TWO / 7.0) * y + TWO / 5.0) * y + TWO / 3.0) * y - x)
        } else {
            const TOL_LOGCF: f64 = 1e-14;
            r * (2.0 * y * logcf(y, 3.0, 2.0, TOL_LOGCF) - x)
        }
    }
}

/// R: nmath/pgamma.c:lgamma1p -- log(gamma(a+1)), accurate for small a
pub fn lgamma1p(a: f64) -> f64 {
    if a.abs() >= 0.5 {
        return lgammafn(a + 1.0);
    }
    const EULERS_CONST: f64 = 0.5772156649015328606065120900824024;
    const N: usize = 40;
    const COEFFS: [f64; 40] = [
        0.3224670334241132182362075833230126e-0,
        0.6735230105319809513324605383715000e-1,
        0.2058080842778454787900092413529198e-1,
        0.7385551028673985266273097291406834e-2,
        0.2890510330741523285752988298486755e-2,
        0.1192753911703260977113935692828109e-2,
        0.5096695247430424223356548135815582e-3,
        0.2231547584535793797614188036013401e-3,
        0.9945751278180853371459589003190170e-4,
        0.4492623673813314170020750240635786e-4,
        0.2050721277567069155316650397830591e-4,
        0.9439488275268395903987425104415055e-5,
        0.4374866789907487804181793223952411e-5,
        0.2039215753801366236781900709670839e-5,
        0.9551412130407419832857179772951265e-6,
        0.4492469198764566043294290331193655e-6,
        0.2120718480555466586923135901077628e-6,
        0.1004322482396809960872083050053344e-6,
        0.4769810169363980565760193417246730e-7,
        0.2271109460894316491031998116062124e-7,
        0.1083865921489695409107491757968159e-7,
        0.5183475041970046655121248647057669e-8,
        0.2483674543802478317185008663991718e-8,
        0.1192140140586091207442548202774640e-8,
        0.5731367241678862013330194857961011e-9,
        0.2759522885124233145178149692816341e-9,
        0.1330476437424448948149715720858008e-9,
        0.6422964563838100022082448087644648e-10,
        0.3104424774732227276239215783404066e-10,
        0.1502138408075414217093301048780668e-10,
        0.7275974480239079662504549924814047e-11,
        0.3527742476575915083615072228655483e-11,
        0.1711991790559617908601084114443031e-11,
        0.8315385841420284819798357793954418e-12,
        0.4042200525289440065536008957032895e-12,
        0.1966475631096616490411045679010286e-12,
        0.9573630387838555763782200936508615e-13,
        0.4664076026428374224576492565974577e-13,
        0.2273736960065972320633279596737272e-13,
        0.1109139947083452201658320007192334e-13,
    ];
    const C: f64 = 0.2273736845824652515226821577978691e-12;
    const TOL_LOGCF: f64 = 1e-14;

    let mut lgam = C * logcf(-a / 2.0, (N + 2) as f64, 1.0, TOL_LOGCF);
    for i in (0..N).rev() {
        lgam = COEFFS[i] - a * lgam;
    }
    (a * lgam - EULERS_CONST) * a - log1pmx(a)
}

/// R: nmath/pgamma.c:logspace_add
pub fn logspace_add(logx: f64, logy: f64) -> f64 {
    fmax2(logx, logy) + log1p((-(logx - logy).abs()).exp())
}

/// R: nmath/pgamma.c:logspace_sub
pub fn logspace_sub(logx: f64, logy: f64) -> f64 {
    logx + r_log1_exp(logy - logx)
}

/// R: nmath/dpois.c:dpois_raw
pub fn dpois_raw(x: f64, lambda: f64, give_log: bool) -> f64 {
    const M_SQRT_2PI: f64 = 2.50662827463100050241576528481104525301;
    const X_LRG: f64 = 2.86111748575702815380240589208115399625e+307;
    if lambda == 0.0 {
        return if x == 0.0 { r_d__1(give_log) } else { r_d__0(give_log) };
    }
    if !lambda.is_finite() {
        return r_d__0(give_log);
    }
    if x < 0.0 {
        return r_d__0(give_log);
    }
    if x <= lambda * DBL_MIN {
        return r_d_exp(-lambda, give_log);
    }
    if lambda < x * DBL_MIN {
        if !x.is_finite() {
            return r_d__0(give_log);
        }
        return r_d_exp(-lambda + x * lambda.ln() - lgammafn(x + 1.0), give_log);
    }
    let (yh, mut yl) = ebd0(x, lambda);
    yl += stirlerr(x);
    let lrg_x = x >= X_LRG;
    let r = if lrg_x { M_SQRT_2PI * x.sqrt() } else { M_2PI * x };
    if give_log {
        -yl - yh - if lrg_x { r.ln() } else { 0.5 * r.ln() }
    } else {
        (-yl).exp() * (-yh).exp() / if lrg_x { r } else { r.sqrt() }
    }
}

/// R: nmath/dpois.c:dpois
pub fn dpois(x: f64, lambda: f64, give_log: bool) -> f64 {
    if x.is_nan() || lambda.is_nan() {
        return x + lambda;
    }
    if lambda < 0.0 {
        return ML_NAN;
    }
    if let Some(v) = r_d_nonint_check(x, give_log) {
        return v;
    }
    if x < 0.0 || !x.is_finite() {
        return r_d__0(give_log);
    }
    dpois_raw(r_forceint(x), lambda, give_log)
}

fn dpois_wrap(x_plus_1: f64, lambda: f64, give_log: bool) -> f64 {
    if !lambda.is_finite() {
        return r_d__0(give_log);
    }
    if x_plus_1 > 1.0 {
        return dpois_raw(x_plus_1 - 1.0, lambda, give_log);
    }
    if lambda > (x_plus_1 - 1.0).abs() * M_CUTOFF {
        r_d_exp(-lambda - lgammafn(x_plus_1), give_log)
    } else {
        let d = dpois_raw(x_plus_1, lambda, give_log);
        if give_log {
            d + (x_plus_1 / lambda).ln()
        } else {
            d * (x_plus_1 / lambda)
        }
    }
}

fn pgamma_smallx(x: f64, alph: f64, lower_tail: bool, log_p: bool) -> f64 {
    let mut sum = 0.0;
    let mut c = alph;
    let mut n = 0.0;
    loop {
        n += 1.0;
        c *= -x / n;
        let term = c / (alph + n);
        sum += term;
        // do { } while (fabs(term) > DBL_EPSILON * fabs(sum)): NaN ends the loop
        if !(term.abs() > DBL_EPSILON * sum.abs()) {
            break;
        }
    }
    if lower_tail {
        let f1 = if log_p { log1p(sum) } else { 1.0 + sum };
        let f2;
        if alph > 1.0 {
            let d = dpois_raw(alph, x, log_p);
            f2 = if log_p { d + x } else { d * x.exp() };
        } else if log_p {
            f2 = alph * x.ln() - lgamma1p(alph);
        } else {
            f2 = x.powf(alph) / lgamma1p(alph).exp();
        }
        if log_p {
            f1 + f2
        } else {
            f1 * f2
        }
    } else {
        let lf2 = alph * x.ln() - lgamma1p(alph);
        if log_p {
            r_log1_exp(log1p(sum) + lf2)
        } else {
            let f1m1 = sum;
            let f2m1 = expm1(lf2);
            -(f1m1 + f2m1 + f1m1 * f2m1)
        }
    }
}

fn pd_upper_series(x: f64, mut y: f64, log_p: bool) -> f64 {
    let mut term = x / y;
    let mut sum = term;
    loop {
        y += 1.0;
        term *= x / y;
        sum += term;
        if !(term > sum * DBL_EPSILON) {
            break;
        }
    }
    if log_p {
        sum.ln()
    } else {
        sum
    }
}

fn pd_lower_cf(y: f64, d: f64) -> f64 {
    const MAX_IT: f64 = 200000.0;
    let mut f = 0.0;
    if y == 0.0 {
        return 0.0;
    }
    let mut f0 = y / d;
    if (y - 1.0).abs() < d.abs() * DBL_EPSILON {
        return f0;
    }
    if f0 > 1.0 {
        f0 = 1.0;
    }
    let mut c2 = y;
    let mut c4 = d;
    let mut a1 = 0.0;
    let mut b1 = 1.0;
    let mut a2 = y;
    let mut b2 = d;

    while b2 > SCALEFACTOR {
        a1 /= SCALEFACTOR;
        b1 /= SCALEFACTOR;
        a2 /= SCALEFACTOR;
        b2 /= SCALEFACTOR;
    }
    let mut i = 0.0;
    let mut of = -1.0;
    while i < MAX_IT {
        i += 1.0;
        c2 -= 1.0;
        let mut c3 = i * c2;
        c4 += 2.0;
        a1 = c4 * a2 + c3 * a1;
        b1 = c4 * b2 + c3 * b1;

        i += 1.0;
        c2 -= 1.0;
        c3 = i * c2;
        c4 += 2.0;
        a2 = c4 * a1 + c3 * a2;
        b2 = c4 * b1 + c3 * b2;

        if b2 > SCALEFACTOR {
            a1 /= SCALEFACTOR;
            b1 /= SCALEFACTOR;
            a2 /= SCALEFACTOR;
            b2 /= SCALEFACTOR;
        }
        if b2 != 0.0 {
            f = a2 / b2;
            if (f - of).abs() <= DBL_EPSILON * fmax2(f0, f.abs()) {
                return f;
            }
            of = f;
        }
    }
    mathlib_warning(format!(" ** NON-convergence in pgamma()'s pd_lower_cf() f= {}.\n", fmt_g(f)));
    f
}

fn pd_lower_series(lambda: f64, mut y: f64) -> f64 {
    let mut term = 1.0;
    let mut sum = 0.0;
    while y >= 1.0 && term > sum * DBL_EPSILON {
        term *= y / lambda;
        sum += term;
        y -= 1.0;
    }
    if y != y.floor() {
        let f = pd_lower_cf(y, lambda + 1.0 - y);
        sum += term * f;
    }
    sum
}

fn dpnorm(mut x: f64, mut lower_tail: bool, lp: f64) -> f64 {
    if x < 0.0 {
        x = -x;
        lower_tail = !lower_tail;
    }
    if x > 10.0 && !lower_tail {
        let mut term = 1.0 / x;
        let mut sum = term;
        let x2 = x * x;
        let mut i = 1.0;
        loop {
            term *= -i / x2;
            sum += term;
            i += 2.0;
            if !(term.abs() > DBL_EPSILON * sum) {
                break;
            }
        }
        1.0 / sum
    } else {
        let d = dnorm(x, 0.0, 1.0, false);
        d / lp.exp()
    }
}

fn ppois_asymp(x: f64, lambda: f64, lower_tail: bool, log_p: bool) -> f64 {
    const COEFS_A: [f64; 8] = [
        -1e99,
        2.0 / 3.0,
        -4.0 / 135.0,
        8.0 / 2835.0,
        16.0 / 8505.0,
        -8992.0 / 12629925.0,
        -334144.0 / 492567075.0,
        698752.0 / 1477701225.0,
    ];
    const COEFS_B: [f64; 8] = [
        -1e99,
        1.0 / 12.0,
        1.0 / 288.0,
        -139.0 / 51840.0,
        -571.0 / 2488320.0,
        163879.0 / 209018880.0,
        5246819.0 / 75246796800.0,
        -534703531.0 / 902961561600.0,
    ];
    let dfm = lambda - x;
    let pt_ = -log1pmx(dfm / x);
    let mut s2pt = (2.0 * x * pt_).sqrt();
    if dfm < 0.0 {
        s2pt = -s2pt;
    }
    let mut res12 = 0.0;
    let mut res1_term = x.sqrt();
    let mut res1_ig = res1_term;
    let mut res2_term = s2pt;
    let mut res2_ig = res2_term;
    for i in 1..8 {
        res12 += res1_ig * COEFS_A[i];
        res12 += res2_ig * COEFS_B[i];
        res1_term *= pt_ / i as f64;
        res2_term *= 2.0 * pt_ / (2 * i + 1) as f64;
        res1_ig = res1_ig / x + res1_term;
        res2_ig = res2_ig / x + res2_term;
    }
    let mut elfb = x;
    let mut elfb_term = 1.0;
    for b in COEFS_B.iter().skip(1) {
        elfb += elfb_term * b;
        elfb_term /= x;
    }
    if !lower_tail {
        elfb = -elfb;
    }
    let f = res12 / elfb;
    let np = pnorm(s2pt, 0.0, 1.0, !lower_tail, log_p);
    if log_p {
        let n_d_over_p = dpnorm(s2pt, !lower_tail, np);
        np + log1p(f * n_d_over_p)
    } else {
        let nd = dnorm(s2pt, 0.0, 1.0, log_p);
        np + f * nd
    }
}

/// R: nmath/pgamma.c:pgamma_raw
pub fn pgamma_raw(x: f64, alph: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x <= 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    if x >= ML_POSINF {
        return r_dt_1(lower_tail, log_p);
    }
    let res;
    if x < 1.0 {
        res = pgamma_smallx(x, alph, lower_tail, log_p);
    } else if x <= alph - 1.0 && x < 0.8 * (alph + 50.0) {
        let sum = pd_upper_series(x, alph, log_p);
        let d = dpois_wrap(alph, x, log_p);
        res = if !lower_tail {
            if log_p {
                r_log1_exp(d + sum)
            } else {
                1.0 - d * sum
            }
        } else if log_p {
            sum + d
        } else {
            sum * d
        };
    } else if alph - 1.0 < x && alph < 0.8 * (x + 50.0) {
        let sum;
        let d = dpois_wrap(alph, x, log_p);
        if alph < 1.0 {
            if x * DBL_EPSILON > 1.0 - alph {
                sum = r_d__1(log_p);
            } else {
                let f = pd_lower_cf(alph, x - (alph - 1.0)) * x / alph;
                sum = if log_p { f.ln() } else { f };
            }
        } else {
            let s = pd_lower_series(x, alph - 1.0);
            sum = if log_p { log1p(s) } else { 1.0 + s };
        }
        res = if !lower_tail {
            if log_p {
                sum + d
            } else {
                sum * d
            }
        } else if log_p {
            r_log1_exp(d + sum)
        } else {
            1.0 - d * sum
        };
    } else {
        res = ppois_asymp(alph - 1.0, x, !lower_tail, log_p);
    }
    if !log_p && res < DBL_MIN / DBL_EPSILON {
        pgamma_raw(x, alph, lower_tail, true).exp()
    } else {
        res
    }
}

/// R: nmath/pgamma.c:pgamma
pub fn pgamma(x: f64, alph: f64, scale: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || alph.is_nan() || scale.is_nan() {
        return x + alph + scale;
    }
    if alph < 0.0 || scale <= 0.0 {
        return ML_NAN;
    }
    let x = x / scale;
    if x.is_nan() {
        return x;
    }
    if alph == 0.0 {
        return if x <= 0.0 { r_dt_0(lower_tail, log_p) } else { r_dt_1(lower_tail, log_p) };
    }
    pgamma_raw(x, alph, lower_tail, log_p)
}

/// R: nmath/dgamma.c:dgamma
pub fn dgamma(x: f64, shape: f64, scale: f64, give_log: bool) -> f64 {
    if x.is_nan() || shape.is_nan() || scale.is_nan() {
        return x + shape + scale;
    }
    if shape < 0.0 || scale <= 0.0 {
        return ML_NAN;
    }
    if x < 0.0 {
        return r_d__0(give_log);
    }
    if shape == 0.0 {
        return if x == 0.0 { ML_POSINF } else { r_d__0(give_log) };
    }
    if x == 0.0 {
        if shape < 1.0 {
            return ML_POSINF;
        }
        if shape > 1.0 {
            return r_d__0(give_log);
        }
        return if give_log { -scale.ln() } else { 1.0 / scale };
    }
    if shape < 1.0 {
        let pr = dpois_raw(shape, x / scale, give_log);
        return if give_log {
            pr + if (shape / x).is_finite() { (shape / x).ln() } else { shape.ln() - x.ln() }
        } else {
            pr * shape / x
        };
    }
    let pr = dpois_raw(shape - 1.0, x / scale, give_log);
    if give_log {
        pr - scale.ln()
    } else {
        pr / scale
    }
}

/// R: nmath/qgamma.c:qchisq_appr
pub fn qchisq_appr(p: f64, nu: f64, g: f64, lower_tail: bool, log_p: bool, tol: f64) -> f64 {
    const C7: f64 = 4.67;
    const C8: f64 = 6.66;
    const C9: f64 = 6.73;
    const C10: f64 = 13.32;

    if p.is_nan() || nu.is_nan() {
        return p + nu;
    }
    if r_q_p01_check(p, log_p) {
        return ML_NAN;
    }
    if nu <= 0.0 {
        return ML_NAN;
    }
    let alpha = 0.5 * nu;
    let c = alpha - 1.0;
    let mut ch;
    let p1 = r_dt_log(p, lower_tail, log_p);
    if nu < (-1.24) * p1 {
        let lgam1pa = if alpha < 0.5 { lgamma1p(alpha) } else { alpha.ln() + g };
        ch = ((lgam1pa + p1) / alpha + M_LN2).exp();
    } else if nu > 0.32 {
        let x = qnorm(p, 0.0, 1.0, lower_tail, log_p);
        let p1 = 2.0 / (9.0 * nu);
        ch = nu * (x * p1.sqrt() + 1.0 - p1).powf(3.0);
        if ch > 2.2 * nu + 6.0 {
            ch = -2.0 * (r_dt_clog(p, lower_tail, log_p) - c * (0.5 * ch).ln() + g);
        }
    } else {
        ch = 0.4;
        let a = r_dt_clog(p, lower_tail, log_p) + g + c * M_LN2;
        loop {
            let q = ch;
            let p1 = 1.0 / (1.0 + ch * (C7 + ch));
            let p2 = ch * (C9 + ch * (C8 + ch));
            let t = -0.5 + (C7 + 2.0 * ch) * p1 - (C9 + ch * (C10 + 3.0 * ch)) / p2;
            ch -= (1.0 - (a + 0.5 * ch).exp() * p2 * p1) / t;
            if !((q - ch).abs() > tol * ch.abs()) {
                break;
            }
        }
    }
    ch
}

/// R: nmath/qgamma.c:qgamma
pub fn qgamma(p: f64, alpha: f64, scale: f64, lower_tail: bool, log_p: bool) -> f64 {
    const EPS1: f64 = 1e-2;
    const EPS2: f64 = 5e-7;
    const EPS_N: f64 = 1e-15;
    const MAXIT: i32 = 1000;
    const P_MIN: f64 = 1e-100;
    const P_MAX: f64 = 1.0 - 1e-14;
    const I420: f64 = 1.0 / 420.0;
    const I2520: f64 = 1.0 / 2520.0;
    const I5040: f64 = 1.0 / 5040.0;

    if p.is_nan() || alpha.is_nan() || scale.is_nan() {
        return p + alpha + scale;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, ML_POSINF, lower_tail, log_p) {
        return v;
    }
    if alpha < 0.0 || scale <= 0.0 {
        return ML_NAN;
    }
    if alpha == 0.0 {
        return 0.0;
    }
    let mut max_it_newton = 1;
    if alpha < 1e-10 {
        max_it_newton = 7;
    }
    let mut p_ = r_dt_qiv(p, lower_tail, log_p);
    let mut g = lgammafn(alpha);

    let mut ch = qchisq_appr(p, 2.0 * alpha, g, lower_tail, log_p, EPS1);
    'phase2: {
        if !ch.is_finite() {
            max_it_newton = 0;
            break 'phase2;
        }
        if ch < EPS2 {
            max_it_newton = 20;
            break 'phase2;
        }
        if p_ > P_MAX || p_ < P_MIN {
            max_it_newton = 20;
            break 'phase2;
        }
        let c = alpha - 1.0;
        let s6 = (120.0 + c * (346.0 + 127.0 * c)) * I5040;
        let ch0 = ch;
        for _ in 1..=MAXIT {
            let q = ch;
            let p1 = 0.5 * ch;
            let p2 = p_ - pgamma_raw(p1, alpha, true, false);
            if !p2.is_finite() || ch <= 0.0 {
                ch = ch0;
                max_it_newton = 27;
                break 'phase2;
            }
            let t = p2 * (alpha * M_LN2 + g + p1 - c * ch.ln()).exp();
            let b = t / ch;
            let a = 0.5 * t - b * c;
            let s1 = (210.0 + a * (140.0 + a * (105.0 + a * (84.0 + a * (70.0 + 60.0 * a))))) * I420;
            let s2 = (420.0 + a * (735.0 + a * (966.0 + a * (1141.0 + 1278.0 * a)))) * I2520;
            let s3 = (210.0 + a * (462.0 + a * (707.0 + 932.0 * a))) * I2520;
            let s4 = (252.0 + a * (672.0 + 1182.0 * a) + c * (294.0 + a * (889.0 + 1740.0 * a))) * I5040;
            let s5 = (84.0 + 2264.0 * a + c * (1175.0 + 606.0 * a)) * I2520;
            ch += t * (1.0 + 0.5 * t * s1 - b * c * (s1 - b * (s2 - b * (s3 - b * (s4 - b * (s5 - b * s6))))));
            if (q - ch).abs() < EPS2 * ch {
                break 'phase2;
            }
            if (q - ch).abs() > 0.1 * ch {
                if ch < q {
                    ch = 0.9 * q;
                } else {
                    ch = 1.1 * q;
                }
            }
        }
    }
    // END:
    let mut x = 0.5 * scale * ch;
    if max_it_newton != 0 {
        let mut p = p;
        let mut log_p = log_p;
        if !log_p {
            p = p.ln();
            log_p = true;
        }
        if x == 0.0 {
            let one_p = 1.0 + 1e-7;
            let one_m = 1.0 - 1e-7;
            x = DBL_MIN;
            p_ = pgamma(x, alpha, scale, lower_tail, log_p);
            if (lower_tail && p_ > p * one_p) || (!lower_tail && p_ < p * one_m) {
                return 0.0;
            }
        } else {
            p_ = pgamma(x, alpha, scale, lower_tail, log_p);
        }
        if p_ == ML_NEGINF {
            return 0.0;
        }
        for i in 1..=max_it_newton {
            let p1 = p_ - p;
            if p1.abs() < (EPS_N * p).abs() {
                break;
            }
            g = dgamma(x, alpha, scale, log_p);
            if g == r_d__0(log_p) {
                break;
            }
            let mut t = if log_p { p1 * (p_ - g).exp() } else { p1 / g };
            t = if lower_tail { x - t } else { x + t };
            p_ = pgamma(t, alpha, scale, lower_tail, log_p);
            if (p_ - p).abs() > p1.abs() || (i > 1 && (p_ - p).abs() == p1.abs()) {
                break;
            }
            x = t;
        }
    }
    x
}

/// R: nmath/ppois.c:ppois
pub fn ppois(x: f64, lambda: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || lambda.is_nan() {
        return x + lambda;
    }
    if lambda < 0.0 {
        return ML_NAN;
    }
    if x < 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    if lambda == 0.0 {
        return r_dt_1(lower_tail, log_p);
    }
    if !x.is_finite() {
        return r_dt_1(lower_tail, log_p);
    }
    let x = (x + 1e-7).floor();
    pgamma(lambda, x + 1.0, 1.0, !lower_tail, log_p)
}

/// R: nmath/qpois.c:qpois
pub fn qpois(p: f64, lambda: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || lambda.is_nan() {
        return p + lambda;
    }
    if !lambda.is_finite() {
        return ML_NAN;
    }
    if lambda < 0.0 {
        return ML_NAN;
    }
    if r_q_p01_check(p, log_p) {
        return ML_NAN;
    }
    if lambda == 0.0 {
        return 0.0;
    }
    if p == r_dt_0(lower_tail, log_p) {
        return 0.0;
    }
    if p == r_dt_1(lower_tail, log_p) {
        return ML_POSINF;
    }
    let mu = lambda;
    let sigma = lambda.sqrt();
    let gamma = 1.0 / sigma;
    discrete_quantile(p, mu, sigma, gamma, None, lower_tail, log_p, &|y, lt, lg| ppois(y, lambda, lt, lg))
}

/// R: nmath/dchisq.c
pub fn dchisq(x: f64, df: f64, give_log: bool) -> f64 {
    dgamma(x, df / 2.0, 2.0, give_log)
}

/// R: nmath/pchisq.c
pub fn pchisq(x: f64, df: f64, lower_tail: bool, log_p: bool) -> f64 {
    pgamma(x, df / 2.0, 2.0, lower_tail, log_p)
}

/// R: nmath/qchisq.c
pub fn qchisq(p: f64, df: f64, lower_tail: bool, log_p: bool) -> f64 {
    qgamma(p, 0.5 * df, 2.0, lower_tail, log_p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamma_pois_values() {
        // GNU R 4.5.0, sprintf("%.17g", ...)
        assert_eq!(pgamma(2.0, 3.0, 1.0, true, false), 0.3233235838169366);
        assert_eq!(pgamma(0.5, 0.2, 1.0, true, false), 0.87877483303620441);
        assert_eq!(pgamma(100.0, 120.0, 1.0, false, false), 0.97176960603513429);
        assert_eq!(qgamma(0.3, 2.5, 1.0, true, false), 1.4999540663799533);
        assert_eq!(dgamma(2.0, 3.0, 1.0, false), 0.2706705664732254);
        assert_eq!(dpois(3.0, 2.5, false), 0.21376301724973648);
        assert_eq!(ppois(3.0, 2.5, true, false), 0.75757613313306593);
        assert_eq!(qpois(0.9, 2.5, true, false), 5.0);
        assert_eq!(pchisq(3.84, 1.0, true, false), 0.94995647875129474);
        assert_eq!(qchisq(0.95, 3.0, true, false), 7.8147279032511792);
    }
}
