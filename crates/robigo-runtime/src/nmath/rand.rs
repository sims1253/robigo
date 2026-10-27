//! Random variate generators: nmath/r*.c. They draw from an [`Rng`].
//!
//! Several C generators (rbinom, rpois, rgamma, rbeta) cache set-up
//! values in statics keyed by their parameters; the cached values are
//! pure functions of the parameters, so recomputing them gives the same
//! streams.

use super::binom::qbinom;
use super::rng::Rng;
use super::*;

/// R: nmath/rnorm.c
pub fn rnorm(rng: &mut Rng, mu: f64, sigma: f64) -> f64 {
    if mu.is_nan() || !sigma.is_finite() || sigma < 0.0 {
        return ML_NAN;
    }
    if sigma == 0.0 || !mu.is_finite() { mu } else { mu + sigma * rng.norm_rand() }
}

/// R: nmath/runif.c
pub fn runif(rng: &mut Rng, a: f64, b: f64) -> f64 {
    if !a.is_finite() || !b.is_finite() || b < a {
        return ML_NAN;
    }
    if a == b {
        a
    } else {
        let mut u;
        loop {
            u = rng.unif_rand();
            if !(u <= 0.0 || u >= 1.0) {
                break;
            }
        }
        a + (b - a) * u
    }
}

/// R: nmath/rexp.c
pub fn rexp(rng: &mut Rng, scale: f64) -> f64 {
    if !scale.is_finite() || scale <= 0.0 {
        if scale == 0.0 {
            return 0.0;
        }
        return ML_NAN;
    }
    scale * rng.exp_rand()
}

/// R: nmath/rbinom.c
pub fn rbinom(rng: &mut Rng, nin: f64, pp: f64) -> f64 {
    if !nin.is_finite() {
        return ML_NAN;
    }
    let r = r_forceint(nin);
    if r != nin {
        return ML_NAN;
    }
    if !pp.is_finite() || r < 0.0 || !(0.0..=1.0).contains(&pp) {
        return ML_NAN;
    }
    if r == 0.0 || pp == 0.0 {
        return 0.0;
    }
    if pp == 1.0 {
        return r;
    }
    if r >= INT_MAX as f64 {
        let u = rng.unif_rand();
        return qbinom(u, r, pp, false, false);
    }
    let n = r as i32;
    let p = fmin2(pp, 1.0 - pp);
    let q = 1.0 - p;
    let np = n as f64 * p;
    let r = p / q;
    let g = r * (n + 1) as f64;
    let psave = pp;

    let mut ix: i32;
    if np < 30.0 {
        let qn = r_pow_di(q, n);
        'small: loop {
            ix = 0;
            let mut f = qn;
            let mut u = rng.unif_rand();
            loop {
                if u < f {
                    break 'small;
                }
                if ix > 110 {
                    break;
                }
                u -= f;
                ix += 1;
                f *= g / ix as f64 - r;
            }
        }
    } else {
        let ffm = np + p;
        let m = ffm as i32;
        let fm = m as f64;
        let npq = np * q;
        let p1 = ((2.195 * npq.sqrt() - 4.6 * q) as i32) as f64 + 0.5;
        let xm = fm + 0.5;
        let xl = xm - p1;
        let xr = xm + p1;
        let c = 0.134 + 20.5 / (15.3 + fm);
        let mut al = (ffm - xl) / (ffm - xl * p);
        let xll = al * (1.0 + 0.5 * al);
        al = (xr - ffm) / (xr * q);
        let xlr = al * (1.0 + 0.5 * al);
        let p2 = p1 * (1.0 + c + c);
        let p3 = p2 + c / xll;
        let p4 = p3 + c / xlr;
        loop {
            let u = rng.unif_rand() * p4;
            let mut v = rng.unif_rand();
            if u <= p1 {
                ix = (xm - p1 * v + u) as i32;
                break;
            }
            if u <= p2 {
                let x = xl + (u - p1) / c;
                v = v * c + 1.0 - (xm - x).abs() / p1;
                if v > 1.0 || v <= 0.0 {
                    continue;
                }
                ix = x as i32;
            } else if u > p3 {
                ix = (xr - v.ln() / xlr) as i32;
                if ix > n {
                    continue;
                }
                v = v * (u - p3) * xlr;
            } else {
                ix = (xl + v.ln() / xll) as i32;
                if ix < 0 {
                    continue;
                }
                v = v * (u - p2) * xll;
            }
            let k = (ix - m).abs();
            if k <= 20 || k as f64 >= npq / 2.0 - 1.0 {
                let mut f = 1.0;
                if m < ix {
                    for i in (m + 1)..=ix {
                        f *= g / i as f64 - r;
                    }
                } else if m != ix {
                    for i in (ix + 1)..=m {
                        f /= g / i as f64 - r;
                    }
                }
                if v <= f {
                    break;
                }
            } else {
                let k = k as f64;
                let amaxp = (k / npq) * ((k * (k / 3.0 + 0.625) + 0.1666666666666) / npq + 0.5);
                let ynorm = -k * k / (2.0 * npq);
                let alv = v.ln();
                if alv < ynorm - amaxp {
                    break;
                }
                if alv <= ynorm + amaxp {
                    let x1 = (ix + 1) as f64;
                    let f1 = fm + 1.0;
                    let z = (n + 1) as f64 - fm;
                    let w = (n - ix) as f64 + 1.0;
                    let z2 = z * z;
                    let x2 = x1 * x1;
                    let f2 = f1 * f1;
                    let w2 = w * w;
                    if alv
                        <= xm * (f1 / x1).ln()
                            + ((n - m) as f64 + 0.5) * (z / w).ln()
                            + (ix - m) as f64 * (w * p / (x1 * q)).ln()
                            + (13860.0 - (462.0 - (132.0 - (99.0 - 140.0 / f2) / f2) / f2) / f2) / f1 / 166320.0
                            + (13860.0 - (462.0 - (132.0 - (99.0 - 140.0 / z2) / z2) / z2) / z2) / z / 166320.0
                            + (13860.0 - (462.0 - (132.0 - (99.0 - 140.0 / x2) / x2) / x2) / x2) / x1 / 166320.0
                            + (13860.0 - (462.0 - (132.0 - (99.0 - 140.0 / w2) / w2) / w2) / w2) / w / 166320.0
                    {
                        break;
                    }
                }
            }
        }
    }
    if psave > 0.5 {
        ix = n - ix;
    }
    ix as f64
}

/// R: nmath/rpois.c
pub fn rpois(rng: &mut Rng, mu: f64) -> f64 {
    const A0: f64 = -0.5;
    const A1: f64 = 0.3333333;
    const A2: f64 = -0.2500068;
    const A3: f64 = 0.2000118;
    const A4: f64 = -0.1661269;
    const A5: f64 = 0.1421878;
    const A6: f64 = -0.1384794;
    const A7: f64 = 0.1250060;
    const ONE_7: f64 = 0.1428571428571428571;
    const ONE_12: f64 = 0.0833333333333333333;
    const ONE_24: f64 = 0.0416666666666666667;
    const FACT: [f64; 10] = [1., 1., 2., 6., 24., 120., 720., 5040., 40320., 362880.];

    if !mu.is_finite() || mu < 0.0 {
        return ML_NAN;
    }
    if mu <= 0.0 {
        return 0.0;
    }
    let big_mu = mu >= 10.0;
    if !big_mu {
        // Case B: table lookup / inversion
        let m = imax2(1, mu as i32);
        let mut l: i32 = 0;
        let p0 = (-mu).exp();
        let mut q = p0;
        let mut p = p0;
        let mut pp = [0.0f64; 36];
        loop {
            let u = rng.unif_rand();
            if u <= p0 {
                return 0.0;
            }
            if l != 0 {
                let start = if u <= 0.458 { 1 } else { imin2(l, m) };
                for k in start..=l {
                    if u <= pp[k as usize] {
                        return k as f64;
                    }
                }
                if l == 35 {
                    continue;
                }
            }
            l += 1;
            for k in l..=35 {
                p *= mu / k as f64;
                q += p;
                pp[k as usize] = q;
                if u <= q {
                    return k as f64;
                }
            }
            l = 35;
        }
    }

    // Case A: mu >= 10
    let s = mu.sqrt();
    let d = 6.0 * mu * mu;
    let big_l = (mu - 1.1484).floor();

    let mut pois = -1.0;
    let mut fk = 0.0;
    let mut difmuk = 0.0;
    let mut u = 0.0;

    let g = mu + s * rng.norm_rand();
    if g >= 0.0 {
        pois = g.floor();
        if pois >= big_l {
            return pois;
        }
        fk = pois;
        difmuk = mu - fk;
        u = rng.unif_rand();
        if d * u >= difmuk * difmuk * difmuk {
            return pois;
        }
    }

    let omega = M_1_SQRT_2PI / s;
    let b1 = ONE_24 / mu;
    let b2 = 0.3 * b1 * b1;
    let c3 = ONE_7 * b1 * b2;
    let c2 = b2 - 15.0 * c3;
    let c1 = b1 - 6.0 * b2 + 45.0 * c3;
    let c0 = 1.0 - b1 + 3.0 * b2 - 15.0 * c3;
    let c = 0.1069 / mu;

    // 'subroutine' F: (px, py, fx, fy)
    let step_f = |pois: f64, fk: f64, difmuk: f64| -> (f64, f64, f64, f64) {
        let (px, py);
        if pois < 10.0 {
            px = -mu;
            py = mu.powf(pois) / FACT[pois as i32 as usize];
        } else {
            let mut del = ONE_12 / fk;
            del = del * (1.0 - 4.8 * del * del);
            let v = difmuk / fk;
            if v.abs() <= 0.25 {
                px = fk * v * v * (((((((A7 * v + A6) * v + A5) * v + A4) * v + A3) * v + A2) * v + A1) * v + A0) - del;
            } else {
                px = fk * (1.0 + v).ln() - difmuk - del;
            }
            py = M_1_SQRT_2PI / fk.sqrt();
        }
        let mut x = (0.5 - difmuk) / s;
        x *= x;
        let fx = -0.5 * x;
        let fy = omega * (((c3 * x + c2) * x + c1) * x + c0);
        (px, py, fx, fy)
    };

    if g >= 0.0 {
        // kflag = 0: Step Q
        let (px, py, fx, fy) = step_f(pois, fk, difmuk);
        if fy - u * fy <= py * (px - fx).exp() {
            return pois;
        }
    }
    loop {
        let e = rng.exp_rand();
        u = 2.0 * rng.unif_rand() - 1.0;
        let t = 1.8 + fsign(e, u);
        if t > -0.6744 {
            pois = (mu + s * t).floor();
            fk = pois;
            difmuk = mu - fk;
            let (px, py, fx, fy) = step_f(pois, fk, difmuk);
            if c * u.abs() <= py * (px + e).exp() - fy * (fx + e).exp() {
                break;
            }
        }
    }
    pois
}

/// R: nmath/rgamma.c
pub fn rgamma(rng: &mut Rng, a: f64, scale: f64) -> f64 {
    const SQRT32: f64 = 5.656854;
    const EXP_M1: f64 = 0.36787944117144232159;
    const Q1: f64 = 0.04166669;
    const Q2: f64 = 0.02083148;
    const Q3: f64 = 0.00801191;
    const Q4: f64 = 0.00144121;
    const Q5: f64 = -7.388e-5;
    const Q6: f64 = 2.4511e-4;
    const Q7: f64 = 2.424e-4;
    const A1: f64 = 0.3333333;
    const A2: f64 = -0.250003;
    const A3: f64 = 0.2000062;
    const A4: f64 = -0.1662921;
    const A5: f64 = 0.1423657;
    const A6: f64 = -0.1367177;
    const A7: f64 = 0.1233795;

    if a.is_nan() || scale.is_nan() {
        return ML_NAN;
    }
    if a <= 0.0 || scale <= 0.0 {
        if scale == 0.0 || a == 0.0 {
            return 0.0;
        }
        return ML_NAN;
    }
    if !a.is_finite() || !scale.is_finite() {
        return ML_POSINF;
    }

    if a < 1.0 {
        let e = 1.0 + EXP_M1 * a;
        let mut x;
        loop {
            let p = e * rng.unif_rand();
            if p >= 1.0 {
                x = -((e - p) / a).ln();
                if rng.exp_rand() >= (1.0 - a) * x.ln() {
                    break;
                }
            } else {
                x = (p.ln() / a).exp();
                if rng.exp_rand() >= x {
                    break;
                }
            }
        }
        return scale * x;
    }

    let s2 = a - 0.5;
    let s = s2.sqrt();
    let d = SQRT32 - s * 12.0;

    let mut t = rng.norm_rand();
    let mut x = s + 0.5 * t;
    let ret_val = x * x;
    if t >= 0.0 {
        return scale * ret_val;
    }
    let mut u = rng.unif_rand();
    if d * u <= t * t * t {
        return scale * ret_val;
    }

    let r = 1.0 / a;
    let q0 = ((((((Q7 * r + Q6) * r + Q5) * r + Q4) * r + Q3) * r + Q2) * r + Q1) * r;
    let (b, si, c);
    if a <= 3.686 {
        b = 0.463 + s + 0.178 * s2;
        si = 1.235;
        c = 0.195 / s - 0.079 + 0.16 * s;
    } else if a <= 13.022 {
        b = 1.654 + 0.0076 * s2;
        si = 1.68 / s + 0.275;
        c = 0.062 / s + 0.024;
    } else {
        b = 1.77;
        si = 0.75;
        c = 0.1515 / s;
    }

    let mut q;
    if x > 0.0 {
        let v = t / (s + s);
        if v.abs() <= 0.25 {
            q = q0 + 0.5 * t * t * ((((((A7 * v + A6) * v + A5) * v + A4) * v + A3) * v + A2) * v + A1) * v;
        } else {
            q = q0 - s * t + 0.25 * t * t + (s2 + s2) * (1.0 + v).ln();
        }
        if (1.0 - u).ln() <= q {
            return scale * ret_val;
        }
    }

    loop {
        let e = rng.exp_rand();
        u = rng.unif_rand();
        u = u + u - 1.0;
        if u < 0.0 {
            t = b - si * e;
        } else {
            t = b + si * e;
        }
        if t >= -0.71874483771719 {
            let v = t / (s + s);
            if v.abs() <= 0.25 {
                q = q0 + 0.5 * t * t * ((((((A7 * v + A6) * v + A5) * v + A4) * v + A3) * v + A2) * v + A1) * v;
            } else {
                q = q0 - s * t + 0.25 * t * t + (s2 + s2) * (1.0 + v).ln();
            }
            if q > 0.0 {
                let w = expm1(q);
                if c * u.abs() <= w * (e - 0.5 * t * t).exp() {
                    break;
                }
            }
        }
    }
    x = s + 0.5 * t;
    scale * x * x
}

/// R: nmath/rbeta.c
pub fn rbeta(rng: &mut Rng, aa: f64, bb: f64) -> f64 {
    const EXPMAX: f64 = DBL_MAX_EXP as f64 * M_LN2;
    if aa.is_nan() || bb.is_nan() || aa < 0.0 || bb < 0.0 {
        return ML_NAN;
    }
    if !aa.is_finite() && !bb.is_finite() {
        return 0.5;
    }
    if aa == 0.0 && bb == 0.0 {
        return if rng.unif_rand() < 0.5 { 0.0 } else { 1.0 };
    }
    if !aa.is_finite() || bb == 0.0 {
        return 1.0;
    }
    if !bb.is_finite() || aa == 0.0 {
        return 0.0;
    }
    let a = fmin2(aa, bb);
    let b = fmax2(aa, bb);
    let alpha = a + b;

    let v_w = |u1: f64, beta: f64, aa_: f64| -> (f64, f64) {
        let v = beta * (u1 / (1.0 - u1)).ln();
        let w = if v <= EXPMAX {
            let w = aa_ * v.exp();
            if !w.is_finite() { DBL_MAX } else { w }
        } else {
            DBL_MAX
        };
        (v, w)
    };

    if a <= 1.0 {
        let beta = 1.0 / a;
        let delta = 1.0 + b - a;
        let k1 = delta * (0.0138889 + 0.0416667 * a) / (b * beta - 0.777778);
        let k2 = 0.25 + (0.5 + 0.25 / delta) * a;
        let w;
        loop {
            let u1 = rng.unif_rand();
            let u2 = rng.unif_rand();
            let z;
            if u1 < 0.5 {
                let y = u1 * u2;
                z = u1 * y;
                if 0.25 * u2 + z - y >= k1 {
                    continue;
                }
            } else {
                z = u1 * u1 * u2;
                if z <= 0.25 {
                    let (_, ww) = v_w(u1, beta, b);
                    w = ww;
                    break;
                }
                if z >= k2 {
                    continue;
                }
            }
            let (v, ww) = v_w(u1, beta, b);
            if alpha * ((alpha / (a + ww)).ln() + v) - 1.3862944 >= z.ln() {
                w = ww;
                break;
            }
        }
        if aa == a { a / (a + w) } else { w / (a + w) }
    } else {
        let beta = ((alpha - 2.0) / (2.0 * a * b - alpha)).sqrt();
        let gamma = a + 1.0 / beta;
        let mut w;
        loop {
            let u1 = rng.unif_rand();
            let u2 = rng.unif_rand();
            let (v, ww) = v_w(u1, beta, a);
            w = ww;
            let z = u1 * u1 * u2;
            let r = gamma * v - 1.3862944;
            let s = a + r - w;
            if s + 2.609438 >= 5.0 * z {
                break;
            }
            let t = z.ln();
            if s > t {
                break;
            }
            // do { } while (r + alpha * log(alpha / (b + w)) < t)
            if !(r + alpha * (alpha / (b + w)).ln() < t) {
                break;
            }
        }
        if aa != a { b / (b + w) } else { w / (b + w) }
    }
}

/// R: nmath/rchisq.c
pub fn rchisq(rng: &mut Rng, df: f64) -> f64 {
    if !df.is_finite() || df < 0.0 {
        return ML_NAN;
    }
    rgamma(rng, df / 2.0, 2.0)
}

/// R: nmath/rcauchy.c
pub fn rcauchy(rng: &mut Rng, location: f64, scale: f64) -> f64 {
    if location.is_nan() || !scale.is_finite() || scale < 0.0 {
        return ML_NAN;
    }
    if scale == 0.0 || !location.is_finite() { location } else { location + scale * (M_PI * rng.unif_rand()).tan() }
}

/// R: nmath/rf.c
pub fn rf(rng: &mut Rng, n1: f64, n2: f64) -> f64 {
    if n1.is_nan() || n2.is_nan() || n1 <= 0.0 || n2 <= 0.0 {
        return ML_NAN;
    }
    let v1 = if n1.is_finite() { rchisq(rng, n1) / n1 } else { 1.0 };
    let v2 = if n2.is_finite() { rchisq(rng, n2) / n2 } else { 1.0 };
    v1 / v2
}

/// R: nmath/rgeom.c
pub fn rgeom(rng: &mut Rng, p: f64) -> f64 {
    if !p.is_finite() || p <= 0.0 || p > 1.0 {
        return ML_NAN;
    }
    let lambda = rng.exp_rand() * ((1.0 - p) / p);
    rpois(rng, lambda)
}

/// R: nmath/rlnorm.c
pub fn rlnorm(rng: &mut Rng, meanlog: f64, sdlog: f64) -> f64 {
    if meanlog.is_nan() || !sdlog.is_finite() || sdlog < 0.0 {
        return ML_NAN;
    }
    rnorm(rng, meanlog, sdlog).exp()
}

/// R: nmath/rlogis.c
pub fn rlogis(rng: &mut Rng, location: f64, scale: f64) -> f64 {
    if location.is_nan() || !scale.is_finite() {
        return ML_NAN;
    }
    if scale == 0.0 || !location.is_finite() {
        location
    } else {
        let u = rng.unif_rand();
        location + scale * (u / (1.0 - u)).ln()
    }
}

/// R: nmath/rnbinom.c:rnbinom
pub fn rnbinom(rng: &mut Rng, size: f64, prob: f64) -> f64 {
    if !prob.is_finite() || size.is_nan() || size <= 0.0 || prob <= 0.0 || prob > 1.0 {
        return ML_NAN;
    }
    let size = if !size.is_finite() { DBL_MAX / 2.0 } else { size };
    if prob == 1.0 {
        0.0
    } else {
        let g = rgamma(rng, size, (1.0 - prob) / prob);
        rpois(rng, g)
    }
}

/// R: nmath/rnbinom.c:rnbinom_mu
pub fn rnbinom_mu(rng: &mut Rng, size: f64, mu: f64) -> f64 {
    if !mu.is_finite() || size.is_nan() || size <= 0.0 || mu < 0.0 {
        return ML_NAN;
    }
    let size = if !size.is_finite() { DBL_MAX / 2.0 } else { size };
    if mu == 0.0 {
        0.0
    } else {
        let g = rgamma(rng, size, mu / size);
        rpois(rng, g)
    }
}

/// R: nmath/rt.c
pub fn rt(rng: &mut Rng, df: f64) -> f64 {
    if df.is_nan() || df <= 0.0 {
        return ML_NAN;
    }
    if !df.is_finite() {
        rng.norm_rand()
    } else {
        let num = rng.norm_rand();
        num / (rchisq(rng, df) / df).sqrt()
    }
}

/// R: nmath/rweibull.c
pub fn rweibull(rng: &mut Rng, shape: f64, scale: f64) -> f64 {
    if !shape.is_finite() || !scale.is_finite() || shape <= 0.0 || scale <= 0.0 {
        if scale == 0.0 {
            return 0.0;
        }
        return ML_NAN;
    }
    scale * (-rng.unif_rand().ln()).powf(1.0 / shape)
}

/// R: nmath/rnchisq.c
pub fn rnchisq(rng: &mut Rng, df: f64, lambda: f64) -> f64 {
    if df.is_nan() || !lambda.is_finite() || df < 0.0 || lambda < 0.0 {
        return ML_NAN;
    }
    if lambda == 0.0 {
        if df == 0.0 { 0.0 } else { rgamma(rng, df / 2.0, 2.0) }
    } else {
        let mut r = rpois(rng, lambda / 2.0);
        if r > 0.0 {
            r = rchisq(rng, 2.0 * r);
        }
        if df > 0.0 {
            r += rgamma(rng, df / 2.0, 2.0);
        }
        r
    }
}

/// R: nmath/signrank.c:rsignrank
pub fn rsignrank(rng: &mut Rng, n: f64) -> f64 {
    if n.is_nan() {
        return n;
    }
    let n = r_forceint(n);
    if n < 0.0 {
        return ML_NAN;
    }
    if n == 0.0 {
        return 0.0;
    }
    let mut r = 0.0;
    let k = n as i32;
    let mut i = 0;
    while i < k {
        i += 1;
        r += i as f64 * (rng.unif_rand() + 0.5).floor();
    }
    r
}

/// R: nmath/wilcox.c:rwilcox
pub fn rwilcox(rng: &mut Rng, m: f64, n: f64) -> f64 {
    if m.is_nan() || n.is_nan() {
        return m + n;
    }
    let m = r_forceint(m);
    let n = r_forceint(n);
    if m < 0.0 || n < 0.0 {
        return ML_NAN;
    }
    if m == 0.0 || n == 0.0 {
        return 0.0;
    }
    let mut r = 0.0;
    let mut k = (m + n) as i32;
    let mut x: Vec<i32> = (0..k).collect();
    let mut i = 0;
    while (i as f64) < n {
        let j = rng.unif_index(k as f64) as i32 as usize;
        r += x[j] as f64;
        k -= 1;
        x[j] = x[k as usize];
        i += 1;
    }
    r - n * (n - 1.0) / 2.0
}
