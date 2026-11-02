//! The hypergeometric distribution: dhyper.c, phyper.c, qhyper.c and
//! rhyper.c.

use super::binom::dbinom_raw;
use super::gamma::lfastchoose;
use super::rand::rbinom;
use super::rng::Rng;
use super::*;

/// R: nmath/dhyper.c:dhyper
pub fn dhyper(x: f64, r: f64, b: f64, n: f64, give_log: bool) -> f64 {
    if x.is_nan() || r.is_nan() || b.is_nan() || n.is_nan() {
        return x + r + b + n;
    }
    if r_d_neg_i_nonint(r) || r_d_neg_i_nonint(b) || r_d_neg_i_nonint(n) || n > r + b {
        return ML_NAN;
    }
    if x < 0.0 {
        return r_d__0(give_log);
    }
    if let Some(v) = r_d_nonint_check(x, give_log) {
        return v;
    }
    let x = r_forceint(x);
    let r = r_forceint(r);
    let b = r_forceint(b);
    let n = r_forceint(n);

    if n < x || r < x || n - x > b {
        return r_d__0(give_log);
    }
    if n == 0.0 {
        return if x == 0.0 { r_d__1(give_log) } else { r_d__0(give_log) };
    }
    let p = n / (r + b);
    let q = (r + b - n) / (r + b);
    let p1 = dbinom_raw(x, r, p, q, give_log);
    let p2 = dbinom_raw(n - x, b, p, q, give_log);
    let p3 = dbinom_raw(n, r + b, p, q, give_log);
    if give_log { p1 + p2 - p3 } else { p1 * p2 / p3 }
}

use super::ldouble::LDouble;

/// R: nmath/phyper.c:pdhyper -- phyper(x, ..)/dhyper(x, ..) (log scale if
/// log_p), assuming x * (NR + NB) <= n * NR. `sum` and `term` are
/// `long double` in C.
fn pdhyper(mut x: f64, nr: f64, nb: f64, n: f64, log_p: bool) -> f64 {
    let mut sum = LDouble::from_f64(0.0);
    let mut term = LDouble::from_f64(1.0);
    while x > 0.0 && term >= sum * DBL_EPSILON {
        term *= x * (nb - n + x) / (n + 1.0 - x) / (nr + 1.0 - x);
        sum += term;
        x -= 1.0;
    }
    let ss = sum.to_f64();
    if log_p { log1p(ss) } else { 1.0 + ss }
}

/// R: nmath/phyper.c:phyper
pub fn phyper(x: f64, nr: f64, nb: f64, n: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || nr.is_nan() || nb.is_nan() || n.is_nan() {
        return x + nr + nb + n;
    }
    let mut x = (x + 1e-7).floor();
    let mut nr = r_forceint(nr);
    let mut nb = r_forceint(nb);
    let n = r_forceint(n);
    let mut lower_tail = lower_tail;

    if nr < 0.0 || nb < 0.0 || !(nr + nb).is_finite() || n < 0.0 || n > nr + nb {
        return ML_NAN;
    }
    if x * (nr + nb) > n * nr {
        std::mem::swap(&mut nb, &mut nr);
        x = n - x - 1.0;
        lower_tail = !lower_tail;
    }
    if x < 0.0 || x < n - nb {
        return r_dt_0(lower_tail, log_p);
    }
    if x >= nr || x >= n {
        return r_dt_1(lower_tail, log_p);
    }
    let d = dhyper(x, nr, nb, n, log_p);
    if (!log_p && d == 0.0) || (log_p && d == ML_NEGINF) {
        return r_dt_0(lower_tail, log_p);
    }
    let pd = pdhyper(x, nr, nb, n, log_p);
    if log_p { r_dt_log_(d + pd, lower_tail) } else { r_d_lval(d * pd, lower_tail) }
}

/// R: nmath/qhyper.c:qhyper
pub fn qhyper(p: f64, nr: f64, nb: f64, n: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || nr.is_nan() || nb.is_nan() || n.is_nan() {
        return p + nr + nb + n;
    }
    if !p.is_finite() || !nr.is_finite() || !nb.is_finite() || !n.is_finite() {
        return ML_NAN;
    }
    let mut nr = r_forceint(nr);
    let mut nb = r_forceint(nb);
    let big_n = nr + nb;
    let n = r_forceint(n);
    if nr < 0.0 || nb < 0.0 || n < 0.0 || n > big_n {
        return ML_NAN;
    }
    let xstart = fmax2(0.0, n - nb);
    let xend = fmin2(n, nr);
    if let Some(v) = r_q_p01_boundaries(p, xstart, xend, lower_tail, log_p) {
        return v;
    }
    let mut xr = xstart;
    let mut xb = n - xr;
    let small_n = big_n < 1000.0;
    let mut term = lfastchoose(nr, xr) + lfastchoose(nb, xb) - lfastchoose(big_n, n);
    if small_n {
        term = term.exp();
    }
    nr -= xr;
    nb -= xb;

    let mut p = p;
    if !lower_tail || log_p {
        p = r_dt_qiv(p, lower_tail, log_p);
    }
    p *= 1.0 - 1000.0 * DBL_EPSILON;
    let mut sum = if small_n { term } else { term.exp() };

    while sum < p && xr < xend {
        xr += 1.0;
        nb += 1.0;
        if small_n {
            term *= (nr / xr) * (xb / nb);
        } else {
            term += ((nr / xr) * (xb / nb)).ln();
        }
        sum += if small_n { term } else { term.exp() };
        xb -= 1.0;
        nr -= 1.0;
    }
    xr
}

/// R: nmath/rhyper.c:afc -- log(i!)
fn afc(i: i32) -> f64 {
    const AL: [f64; 8] = [
        0.0,
        0.0,
        0.69314718055994530941723212145817,
        1.79175946922805500081247735838070,
        3.17805383034794561964694160129705,
        4.78749174278204599424770093452324,
        6.57925121201010099506017829290394,
        8.52516136106541430016553103634712,
    ];
    if i < 0 {
        mathlib_warning(format!("rhyper.c: afc(i), i={} < 0 -- SHOULD NOT HAPPEN!\n", i));
        return -1.0;
    }
    if i <= 7 {
        return AL[i as usize];
    }
    let di = i as f64;
    let i2 = di * di;
    (di + 0.5) * di.ln() - di + M_LN_SQRT_2PI + (0.0833333333333333 - 0.00277777777777778 / i2) / di
}

/// R: nmath/rhyper.c:rhyper -- NR 'red', NB 'blue', n drawn; how many red.
///
/// The C code caches its set-up in statics keyed by (nn1, nn2, kk); the
/// cached values are pure functions of those parameters (the one static
/// overwritten during sampling, `s`, is only read during set-up), so the
/// set-up is recomputed on every call here with identical results.
pub fn rhyper(rng: &mut Rng, nn1in: f64, nn2in: f64, kkin: f64) -> f64 {
    if !nn1in.is_finite() || !nn2in.is_finite() || !kkin.is_finite() {
        return ML_NAN;
    }
    let nn1in = r_forceint(nn1in);
    let nn2in = r_forceint(nn2in);
    let kkin = r_forceint(kkin);

    if nn1in < 0.0 || nn2in < 0.0 || kkin < 0.0 || kkin > nn1in + nn2in {
        return ML_NAN;
    }
    if nn1in >= INT_MAX as f64 || nn2in >= INT_MAX as f64 || kkin >= INT_MAX as f64 {
        if kkin == 1.0 {
            return rbinom(rng, kkin, nn1in / (nn1in + nn2in));
        }
        let u = rng.unif_rand();
        return qhyper(u, nn1in, nn2in, kkin, false, false);
    }
    let nn1 = nn1in as i32;
    let nn2 = nn2in as i32;
    let kk = kkin as i32;

    // set-up (setup1: n1, n2; setup2: k)
    let big_n = nn1 as f64 + nn2 as f64;
    let (n1, n2) = if nn1 <= nn2 { (nn1, nn2) } else { (nn2, nn1) };
    let k = if kk as f64 + kk as f64 >= big_n { (big_n - kk as f64) as i32 } else { kk };
    let m = ((k as f64 + 1.0) * (n1 as f64 + 1.0) / (big_n + 2.0)) as i32;
    let minjx = imax2(0, k - n2);
    let maxjx = imin2(n1, k);

    let mut ix: i32;
    if minjx == maxjx {
        // I: degenerate distribution
        ix = maxjx;
    } else if m - minjx < 10 {
        // II: (Scaled) algorithm HIN (inverse transformation)
        const SCALE: f64 = 1e25;
        const CON: f64 = 57.5646273248511421;
        let lw = if k < n2 {
            afc(n2) + afc(n1 + n2 - k) - afc(n2 - k) - afc(n1 + n2)
        } else {
            afc(n1) + afc(k) - afc(k - n2) - afc(n1 + n2)
        };
        let w = (lw + CON).exp();
        'l10: loop {
            let mut p = w;
            ix = minjx;
            let mut u = rng.unif_rand() * SCALE;
            while u > p {
                u -= p;
                p *= (n1 as f64 - ix as f64) * (k - ix) as f64;
                ix += 1;
                p = p / ix as f64 / (n2 - k + ix) as f64;
                if ix > maxjx {
                    continue 'l10;
                }
            }
            break;
        }
    } else {
        // III: H2PE Algorithm
        let s = ((big_n - k as f64) * k as f64 * n1 as f64 * n2 as f64 / (big_n - 1.0) / big_n / big_n).sqrt();
        let d = ((1.5 * s) as i32) as f64 + 0.5;
        let xl = m as f64 - d + 0.5;
        let xr = m as f64 + d + 0.5;
        let a = afc(m) + afc(n1 - m) + afc(k - m) + afc(n2 - k + m);
        let kl = (a
            - afc(xl as i32)
            - afc((n1 as f64 - xl) as i32)
            - afc((k as f64 - xl) as i32)
            - afc((n2 as f64 - k as f64 + xl) as i32))
        .exp();
        let kr = (a
            - afc((xr - 1.0) as i32)
            - afc((n1 as f64 - xr + 1.0) as i32)
            - afc((k as f64 - xr + 1.0) as i32)
            - afc((n2 as f64 - k as f64 + xr - 1.0) as i32))
        .exp();
        let lamdl = -(xl * (n2 as f64 - k as f64 + xl) / (n1 as f64 - xl + 1.0) / (k as f64 - xl + 1.0)).ln();
        let lamdr = -((n1 as f64 - xr + 1.0) * (k as f64 - xr + 1.0) / xr / (n2 as f64 - k as f64 + xr)).ln();
        let p1 = d + d;
        let p2 = p1 + kl / lamdl;
        let p3 = p2 + kr / lamdr;

        let mut n_uv = 0;
        loop {
            // L30:
            let u = rng.unif_rand() * p3;
            let mut v = rng.unif_rand();
            n_uv += 1;
            if n_uv >= 10000 {
                // REprintf in C (printed directly to stderr, not a warning)
                eprint!("rhyper(*, n1={}, n2={}, k={}): branch III: giving up after {} rejections", nn1, nn2, kk, n_uv);
                return ML_NAN;
            }
            if u < p1 {
                ix = (xl + u) as i32;
            } else if u <= p2 {
                ix = (xl + v.ln() / lamdl) as i32;
                if ix < minjx {
                    continue;
                }
                v = v * (u - p1) * lamdl;
            } else {
                ix = (xr - v.ln() / lamdr) as i32;
                if ix > maxjx {
                    continue;
                }
                v = v * (u - p2) * lamdr;
            }

            let reject;
            if m < 100 || ix <= 50 {
                // explicit evaluation
                let mut f = 1.0;
                if m < ix {
                    for i in (m + 1)..=ix {
                        f = f * (n1 - i + 1) as f64 * (k - i + 1) as f64 / (n2 - k + i) as f64 / i as f64;
                    }
                } else if m > ix {
                    for i in (ix + 1)..=m {
                        f = f * i as f64 * (n2 - k + i) as f64 / (n1 - i + 1) as f64 / (k - i + 1) as f64;
                    }
                }
                reject = !(v <= f);
            } else {
                const DELTAL: f64 = 0.0078;
                const DELTAU: f64 = 0.0034;
                // squeeze using upper and lower bounds
                let y = ix as f64;
                let y1 = y + 1.0;
                let ym = y - m as f64;
                let yn = n1 as f64 - y + 1.0;
                let yk = k as f64 - y + 1.0;
                let nk = n2 as f64 - k as f64 + y1;
                let r = -ym / y1;
                let s = ym / yn;
                let t = ym / yk;
                let e = -ym / nk;
                let g = yn * yk / (y1 * nk) - 1.0;
                let mut dg = 1.0;
                if g < 0.0 {
                    dg = 1.0 + g;
                }
                let gu = g * (1.0 + g * (-0.5 + g / 3.0));
                let gl = gu - 0.25 * (g * g * g * g) / dg;
                let xm = m as f64 + 0.5;
                let xn = n1 as f64 - m as f64 + 0.5;
                let xk = k as f64 - m as f64 + 0.5;
                let nm = n2 as f64 - k as f64 + xm;
                let ub = y * gu - m as f64 * gl
                    + DELTAU
                    + xm * r * (1. + r * (-0.5 + r / 3.0))
                    + xn * s * (1. + s * (-0.5 + s / 3.0))
                    + xk * t * (1. + t * (-0.5 + t / 3.0))
                    + nm * e * (1. + e * (-0.5 + e / 3.0));
                let alv = v.ln();
                if alv > ub {
                    reject = true;
                } else {
                    let mut dr = xm * (r * r * r * r);
                    if r < 0.0 {
                        dr /= 1.0 + r;
                    }
                    let mut ds = xn * (s * s * s * s);
                    if s < 0.0 {
                        ds /= 1.0 + s;
                    }
                    let mut dt = xk * (t * t * t * t);
                    if t < 0.0 {
                        dt /= 1.0 + t;
                    }
                    let mut de = nm * (e * e * e * e);
                    if e < 0.0 {
                        de /= 1.0 + e;
                    }
                    if alv < ub - 0.25 * (dr + ds + dt + de) + (y + m as f64) * (gl - gu) - DELTAL {
                        reject = false;
                    } else {
                        reject = !(alv <= (a - afc(ix) - afc(n1 - ix) - afc(k - ix) - afc(n2 - k + ix)));
                    }
                }
            }
            if !reject {
                break;
            }
        }
    }

    // L_finis: return appropriate variate
    if kk as f64 + kk as f64 >= big_n {
        if nn1 > nn2 {
            ix += kk - nn2;
        } else {
            ix = nn1 - ix;
        }
    } else if nn1 > nn2 {
        ix = kk - ix;
    }
    ix as f64
}
