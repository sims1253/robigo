//! Bessel functions: bessel_i.c, bessel_j.c, bessel_k.c, bessel_y.c
//! (W. J. Cody's netlib specfun routines, as translated in GNU R) and
//! gamma_cody.c. The `*_vec` functions mirror arithmetic.c's
//! math2B/math3B as used by `.Internal(besselJ(x, nu))` & co.

#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::needless_return)]

use super::*;

// bessel.h
const NSIG_BESS: i32 = 16;
const ENSIG_BESS: f64 = 1e16;
const RTNSIG_BESS: f64 = 1e-4;
const ENMTEN_BESS: f64 = 8.9e-308;
const ENTEN_BESS: f64 = 1e308;
const EXPARG_BESS: f64 = 709.;
const XLRG_BESS_IJ: f64 = 1e5;
const XLRG_BESS_Y: f64 = 1e8;
const THRESH_BESS_Y: f64 = 16.;
const XMAX_BESS_K: f64 = 705.342;
const SQXMIN_BESS_K: f64 = 1.49e-154;
const M_EPS_SINC: f64 = 2.149e-8;

/// arithmetic.c: besselJY_max_nu
const BESSEL_JY_MAX_NU: f64 = 1e7;

#[inline]
fn min0(x: i32, y: i32) -> i32 {
    if x <= y { x } else { y }
}

#[inline]
fn max0(x: i32, y: i32) -> i32 {
    if x <= y { y } else { x }
}

/// R: nmath/gamma_cody.c:Rf_gamma_cody
pub fn gamma_cody(x: f64) -> f64 {
    const SQRTPI: f64 = 0.9189385332046727417803297;
    const XBIG: f64 = 171.624;
    const P: [f64; 8] = [
        -1.71618513886549492533811,
        24.7656508055759199108314,
        -379.804256470945635097577,
        629.331155312818442661052,
        866.966202790413211295064,
        -31451.2729688483675254357,
        -36144.4134186911729807069,
        66456.1438202405440627855,
    ];
    const Q: [f64; 8] = [
        -30.8402300119738975254353,
        315.350626979604161529144,
        -1015.15636749021914166146,
        -3107.77167157231109440444,
        22538.1184209801510330112,
        4755.84627752788110767815,
        -134659.959864969306392456,
        -115132.259675553483497211,
    ];
    const C: [f64; 7] = [
        -0.001910444077728,
        8.4171387781295e-4,
        -5.952379913043012e-4,
        7.93650793500350248e-4,
        -0.002777777777777681622553,
        0.08333333333333333331554247,
        0.0057083835261,
    ];

    let mut parity = false;
    let mut fact = 1.0;
    let mut n = 0;
    let mut y = x;
    let mut res;
    if y <= 0.0 {
        y = -x;
        let yi = y.trunc();
        res = y - yi;
        if res != 0.0 {
            if yi != (yi * 0.5).trunc() * 2.0 {
                parity = true;
            }
            fact = -M_PI / sinpi(res);
            y += 1.0;
        } else {
            return ML_POSINF;
        }
    }
    if y < DBL_EPSILON {
        if y >= DBL_MIN {
            res = 1.0 / y;
        } else {
            return ML_POSINF;
        }
    } else if y < 12.0 {
        let yi = y;
        let z;
        if y < 1.0 {
            z = y;
            y += 1.0;
        } else {
            n = y as i32 - 1;
            y -= n as f64;
            z = y - 1.0;
        }
        let mut xnum = 0.0;
        let mut xden = 1.0;
        for i in 0..8 {
            xnum = (xnum + P[i]) * z;
            xden = xden * z + Q[i];
        }
        res = xnum / xden + 1.0;
        if yi < y {
            res /= yi;
        } else if yi > y {
            for _ in 0..n {
                res *= y;
                y += 1.0;
            }
        }
    } else if y <= XBIG {
        let ysq = y * y;
        let mut sum = C[6];
        for c in C.iter().take(6) {
            sum = sum / ysq + c;
        }
        sum = sum / y - y + SQRTPI;
        sum += (y - 0.5) * y.ln();
        res = sum.exp();
    } else {
        return ML_POSINF;
    }
    if parity {
        res = -res;
    }
    if fact != 1.0 {
        res = fact / res;
    }
    res
}

// ----------------------------------------------------------------------
// J

/// R: nmath/bessel_j.c:bessel_j (allocating its own work array)
pub fn bessel_j(x: f64, alpha: f64) -> f64 {
    if x.is_nan() || alpha.is_nan() {
        return x + alpha;
    }
    if x < 0.0 {
        ml_warning(ME_RANGE, "bessel_j");
        return ML_NAN;
    }
    let na = alpha.floor();
    if alpha < 0.0 {
        return (if alpha - na == 0.5 { 0.0 } else { bessel_j(x, -alpha) * cospi(alpha) })
            + (if alpha == na { 0.0 } else { bessel_y(x, -alpha) * sinpi(alpha) });
    } else if alpha > 1e7 {
        mathlib_warning(format!("besselJ(x, nu): nu={} too large for bessel_j() algorithm", fmt_g(alpha)));
        return ML_NAN;
    }
    let nb = 1 + na as i32;
    let mut bj = vec![0.0; nb as usize];
    bessel_j_core(x, alpha, nb, &mut bj)
}

/// R: nmath/bessel_j.c:bessel_j_ex -- with a caller-supplied work array
/// of length >= 1 + floor(|alpha|).
pub fn bessel_j_ex(x: f64, alpha: f64, bj: &mut [f64]) -> f64 {
    if x.is_nan() || alpha.is_nan() {
        return x + alpha;
    }
    if x < 0.0 {
        ml_warning(ME_RANGE, "bessel_j");
        return ML_NAN;
    }
    let na = alpha.floor();
    if alpha < 0.0 {
        return (if alpha - na == 0.5 { 0.0 } else { bessel_j_ex(x, -alpha, bj) * cospi(alpha) })
            + (if alpha == na { 0.0 } else { bessel_y_ex(x, -alpha, bj) * sinpi(alpha) });
    } else if alpha > 1e7 {
        mathlib_warning(format!("besselJ(x, nu): nu={} too large for bessel_j() algorithm", fmt_g(alpha)));
        return ML_NAN;
    }
    let nb = 1 + na as i32;
    bessel_j_core(x, alpha, nb, bj)
}

fn bessel_j_core(x: f64, alpha: f64, nb: i32, bj: &mut [f64]) -> f64 {
    let alpha = alpha - (nb - 1) as f64;
    let mut ncalc = 0;
    j_bessel(x, alpha, nb, bj, &mut ncalc);
    if ncalc != nb {
        if ncalc < 0 {
            mathlib_warning(format!(
                "bessel_j({}): ncalc (={}) != nb (={}); alpha={}. Arg. out of range?\n",
                fmt_g(x),
                ncalc,
                nb,
                fmt_g(alpha)
            ));
        } else {
            mathlib_warning(format!(
                "bessel_j({},nu={}): precision lost in result\n",
                fmt_g(x),
                fmt_g(alpha + nb as f64 - 1.0)
            ));
        }
    }
    bj[(nb - 1) as usize]
}

/// R: nmath/bessel_j.c:J_bessel. `b` is 0-based here (C: b[1..nb]).
fn j_bessel(x: f64, alpha: f64, nb: i32, bo: &mut [f64], ncalc: &mut i32) {
    const PI2: f64 = 0.636619772367581343075535;
    const TWOPI1: f64 = 6.28125;
    const TWOPI2: f64 = 0.001935307179586476925286767;
    const FACT: [f64; 25] = [
        1.,
        1.,
        2.,
        6.,
        24.,
        120.,
        720.,
        5040.,
        40320.,
        362880.,
        3628800.,
        39916800.,
        479001600.,
        6227020800.,
        87178291200.,
        1.307674368e12,
        2.0922789888e13,
        3.55687428096e14,
        6.402373705728e15,
        1.21645100408832e17,
        2.43290200817664e18,
        5.109094217170944e19,
        1.12400072777760768e21,
        2.585201673888497664e22,
        6.2044840173323943936e23,
    ];

    // 1-based accessors
    macro_rules! b {
        ($i:expr) => {
            bo[($i) as usize - 1]
        };
    }

    let nu = alpha;
    let twonu = nu + nu;

    if nb > 0 && x >= 0.0 && (0.0..1.0).contains(&nu) {
        *ncalc = nb;
        if x > XLRG_BESS_IJ {
            ml_warning(ME_RANGE, "J_bessel");
            for i in 1..=nb {
                b!(i) = 0.0;
            }
            return;
        }
        let intx = x as i32;
        for i in 1..=nb {
            b!(i) = 0.0;
        }

        if x < RTNSIG_BESS {
            // Two-term ascending series for small X.
            let mut alpem = 1.0 + nu;
            let halfx = if x > ENMTEN_BESS { 0.5 * x } else { 0.0 };
            let mut aa = if nu != 0.0 { halfx.powf(nu) / (nu * gamma_cody(nu)) } else { 1.0 };
            let bb = if x + 1.0 > 1.0 { -halfx * halfx } else { 0.0 };
            b!(1) = aa + aa * bb / alpem;
            if x != 0.0 && b!(1) == 0.0 {
                *ncalc = 0;
            }
            if nb != 1 {
                if x <= 0.0 {
                    for n in 2..=nb {
                        b!(n) = 0.0;
                    }
                } else {
                    let tover = if bb == 0.0 { (ENMTEN_BESS + ENMTEN_BESS) / x } else { ENMTEN_BESS / bb };
                    let cc = halfx;
                    for n in 2..=nb {
                        aa /= alpem;
                        alpem += 1.0;
                        aa *= cc;
                        if aa <= tover * alpem {
                            aa = 0.0;
                        }
                        b!(n) = aa + aa * bb / alpem;
                        if b!(n) == 0.0 && *ncalc > n {
                            *ncalc = n - 1;
                        }
                    }
                }
            }
        } else if x > 25.0 && nb <= intx + 1 {
            // Asymptotic series for X > 25 (and not too large nb)
            let xc = (PI2 / x).sqrt();
            let xin = 1.0 / (64.0 * x * x);
            let m: i32 = if x >= 130.0 {
                4
            } else if x >= 35.0 {
                8
            } else {
                11
            };
            let xm = 4.0 * m as f64;
            let mut t = (x / (TWOPI1 + TWOPI2) + 0.5).trunc();
            let z = (x - t * TWOPI1) - t * TWOPI2 - (nu + 0.5) / PI2;
            let mut vsin = z.sin();
            let mut vcos = z.cos();
            let mut gnu = twonu;
            for i in 1..=2 {
                let mut s = (xm - 1.0 - gnu) * (xm - 1.0 + gnu) * xin * 0.5;
                t = (gnu - (xm - 3.0)) * (gnu + (xm - 3.0));
                let mut t1 = (gnu - (xm + 1.0)) * (gnu + (xm + 1.0));
                let mut k = m + m;
                let mut capp = s * t / FACT[k as usize];
                let mut capq = s * t1 / FACT[(k + 1) as usize];
                let mut xk = xm;
                while k >= 4 {
                    xk -= 4.0;
                    s = (xk - 1.0 - gnu) * (xk - 1.0 + gnu);
                    t1 = t;
                    t = (gnu - (xk - 3.0)) * (gnu + (xk - 3.0));
                    capp = (capp + 1.0 / FACT[(k - 2) as usize]) * s * t * xin;
                    capq = (capq + 1.0 / FACT[(k - 1) as usize]) * s * t1 * xin;
                    k -= 2;
                }
                capp += 1.0;
                capq = (capq + 1.0) * (gnu * gnu - 1.0) * (0.125 / x);
                b!(i) = xc * (capp * vcos - capq * vsin);
                if nb == 1 {
                    return;
                }
                t = vsin;
                vsin = -vcos;
                vcos = t;
                gnu += 2.0;
            }
            if nb > 2 {
                let mut gnu = twonu + 2.0;
                for j in 3..=nb {
                    b!(j) = gnu * b!(j - 1) / x - b!(j - 2);
                    gnu += 2.0;
                }
            }
        } else {
            // Use recurrence to generate results.
            let nbmx = nb - intx;
            let mut n = intx + 1;
            let mut en = (n + n) as f64 + twonu;
            let mut plast = 1.0;
            let mut p = en / x;
            let mut pold;
            let mut test = ENSIG_BESS + ENSIG_BESS;
            let mut goto_l190 = false;
            if nbmx >= 3 {
                let mut tover = ENTEN_BESS / ENSIG_BESS;
                let mut nstart = intx + 2;
                let mut nend = nb - 1;
                en = (nstart + nstart) as f64 - 2.0 + twonu;
                let mut k = nstart;
                while k <= nend {
                    n = k;
                    en += 2.0;
                    pold = plast;
                    plast = p;
                    p = en * plast / x - pold;
                    if p > tover {
                        tover = ENTEN_BESS;
                        p /= tover;
                        plast /= tover;
                        let mut psave = p;
                        let mut psavel = plast;
                        nstart = n + 1;
                        loop {
                            n += 1;
                            en += 2.0;
                            pold = plast;
                            plast = p;
                            p = en * plast / x - pold;
                            if !(p <= 1.0) {
                                break;
                            }
                        }
                        let bb = en / x;
                        test = pold * plast * (0.5 - 0.5 / (bb * bb));
                        test /= ENSIG_BESS;
                        p = plast * tover;
                        n -= 1;
                        en -= 2.0;
                        nend = min0(nb, n);
                        let mut found = false;
                        for l in nstart..=nend {
                            pold = psavel;
                            psavel = psave;
                            psave = en * psavel / x - pold;
                            if psave * psavel > test {
                                *ncalc = l - 1;
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            *ncalc = nend;
                        }
                        goto_l190 = true;
                        break;
                    }
                    k += 1;
                }
                if !goto_l190 {
                    n = nend;
                    en = (n + n) as f64 + twonu;
                    test = fmax2(test, (plast * ENSIG_BESS).sqrt() * (p + p).sqrt());
                }
            }
            if !goto_l190 {
                loop {
                    n += 1;
                    en += 2.0;
                    pold = plast;
                    plast = p;
                    p = en * plast / x - pold;
                    if !(p < test) {
                        break;
                    }
                }
            }
            // L190: backward recursion and normalization sum
            n += 1;
            en += 2.0;
            let mut bb = 0.0;
            let mut aa = 1.0 / p;
            let mut m = n / 2;
            let mut em = m as f64;
            m = (n << 1) - (m << 2);
            let mut sum;
            let mut alpem;
            let mut alp2em;
            if m == 0 {
                sum = 0.0;
            } else {
                alpem = em - 1.0 + nu;
                alp2em = em + em + nu;
                sum = aa * alpem * alp2em / em;
            }
            let nend = n - nb;
            for _ in 1..=nend {
                n -= 1;
                en -= 2.0;
                let cc = bb;
                bb = aa;
                aa = en * bb / x - cc;
                m = if m != 0 { 0 } else { 2 };
                if m != 0 {
                    em -= 1.0;
                    alp2em = em + em + nu;
                    if n == 1 {
                        break;
                    }
                    alpem = em - 1.0 + nu;
                    if alpem == 0.0 {
                        alpem = 1.0;
                    }
                    sum = (sum + aa * alp2em) * alpem / em;
                }
            }
            b!(n) = aa;
            // Control flow labels: L240 / L250
            let mut goto_l240 = false;
            let mut goto_l250 = false;
            if nend >= 0 {
                if nb <= 1 {
                    alp2em = if nu + 1.0 == 1.0 { 1.0 } else { nu };
                    sum += b!(1) * alp2em;
                    goto_l250 = true;
                } else {
                    n -= 1;
                    en -= 2.0;
                    b!(n) = en * aa / x - bb;
                    if n == 1 {
                        goto_l240 = true;
                    } else {
                        m = if m != 0 { 0 } else { 2 };
                        if m != 0 {
                            em -= 1.0;
                            alp2em = em + em + nu;
                            alpem = em - 1.0 + nu;
                            if alpem == 0.0 {
                                alpem = 1.0;
                            }
                            sum = (sum + b!(n) * alp2em) * alpem / em;
                        }
                    }
                }
            }
            if !goto_l240 && !goto_l250 {
                n -= 1;
                while n >= 2 {
                    en -= 2.0;
                    b!(n) = en * b!(n + 1) / x - b!(n + 2);
                    m = if m != 0 { 0 } else { 2 };
                    if m != 0 {
                        em -= 1.0;
                        alp2em = em + em + nu;
                        alpem = em - 1.0 + nu;
                        if alpem == 0.0 {
                            alpem = 1.0;
                        }
                        sum = (sum + b!(n) * alp2em) * alpem / em;
                    }
                    n -= 1;
                }
                b!(1) = 2.0 * (nu + 1.0) * b!(2) / x - b!(3);
                goto_l240 = true;
            }
            if goto_l240 {
                em -= 1.0;
                alp2em = em + em + nu;
                if alp2em == 0.0 {
                    alp2em = 1.0;
                }
                sum += b!(1) * alp2em;
            }
            // L250: normalize
            if nu.abs() > 1e-15 {
                sum *= gamma_cody(nu) * (0.5 * x).powf(-nu);
            }
            aa = ENMTEN_BESS;
            if sum > 1.0 {
                aa *= sum;
            }
            for n in 1..=nb {
                if b!(n).abs() < aa {
                    b!(n) = 0.0;
                } else {
                    b!(n) /= sum;
                }
            }
        }
    } else {
        b!(1) = 0.0;
        *ncalc = min0(nb, 0) - 1;
    }
}

// ----------------------------------------------------------------------
// Y

/// R: nmath/bessel_y.c:bessel_y
pub fn bessel_y(x: f64, alpha: f64) -> f64 {
    if x.is_nan() || alpha.is_nan() {
        return x + alpha;
    }
    if x < 0.0 {
        ml_warning(ME_RANGE, "bessel_y");
        return ML_NAN;
    }
    let na = alpha.floor();
    if alpha < 0.0 {
        return (if alpha - na == 0.5 { 0.0 } else { bessel_y(x, -alpha) * cospi(alpha) })
            - (if alpha == na { 0.0 } else { bessel_j(x, -alpha) * sinpi(alpha) });
    } else if alpha > 1e7 {
        mathlib_warning(format!("besselY(x, nu): nu={} too large for bessel_y() algorithm", fmt_g(alpha)));
        return ML_NAN;
    }
    let nb = 1 + na as i32;
    let mut by = vec![0.0; nb as usize];
    bessel_y_core(x, alpha, nb, &mut by)
}

/// R: nmath/bessel_y.c:bessel_y_ex
pub fn bessel_y_ex(x: f64, alpha: f64, by: &mut [f64]) -> f64 {
    if x.is_nan() || alpha.is_nan() {
        return x + alpha;
    }
    if x < 0.0 {
        ml_warning(ME_RANGE, "bessel_y");
        return ML_NAN;
    }
    let na = alpha.floor();
    if alpha < 0.0 {
        return (if alpha - na == 0.5 { 0.0 } else { bessel_y_ex(x, -alpha, by) * cospi(alpha) })
            - (if alpha == na { 0.0 } else { bessel_j_ex(x, -alpha, by) * sinpi(alpha) });
    } else if alpha > 1e7 {
        mathlib_warning(format!("besselY(x, nu): nu={} too large for bessel_y() algorithm", fmt_g(alpha)));
        return ML_NAN;
    }
    let nb = 1 + na as i32;
    bessel_y_core(x, alpha, nb, by)
}

fn bessel_y_core(x: f64, alpha: f64, nb: i32, by: &mut [f64]) -> f64 {
    let alpha = alpha - (nb - 1) as f64;
    let mut ncalc = 0;
    y_bessel(x, alpha, nb, by, &mut ncalc);
    if ncalc != nb {
        if ncalc == -1 {
            return ML_POSINF;
        } else if ncalc < -1 {
            mathlib_warning(format!(
                "bessel_y({}): ncalc (={}) != nb (={}); alpha={}. Arg. out of range?\n",
                fmt_g(x),
                ncalc,
                nb,
                fmt_g(alpha)
            ));
        } else {
            mathlib_warning(format!(
                "bessel_y({},nu={}): precision lost in result\n",
                fmt_g(x),
                fmt_g(alpha + nb as f64 - 1.0)
            ));
        }
    }
    by[(nb - 1) as usize]
}

/// R: nmath/bessel_y.c:Y_bessel (0-based `by` as in C)
fn y_bessel(x: f64, alpha: f64, nb: i32, by: &mut [f64], ncalc: &mut i32) {
    const FIVPI: f64 = 15.707963267948966192;
    const PIM5: f64 = 0.70796326794896619231;
    const CH: [f64; 21] = [
        -6.7735241822398840964e-24,
        -6.1455180116049879894e-23,
        2.9017595056104745456e-21,
        1.3639417919073099464e-19,
        2.3826220476859635824e-18,
        -9.0642907957550702534e-18,
        -1.4943667065169001769e-15,
        -3.3919078305362211264e-14,
        -1.7023776642512729175e-13,
        9.1609750938768647911e-12,
        2.4230957900482704055e-10,
        1.7451364971382984243e-9,
        -3.3126119768180852711e-8,
        -8.6592079961391259661e-7,
        -4.9717367041957398581e-6,
        7.6309597585908126618e-5,
        0.0012719271366545622927,
        0.0017063050710955562222,
        -0.07685284084478667369,
        -0.28387654227602353814,
        0.92187029365045265648,
    ];

    let ex = x;
    let mut nu = alpha;
    let mut ya;
    let mut ya1;
    if nb > 0 && (0.0..1.0).contains(&nu) {
        if ex < DBL_MIN || ex > XLRG_BESS_Y {
            *ncalc = nb;
            if ex > XLRG_BESS_Y {
                by[0] = 0.0;
            } else if ex < DBL_MIN {
                by[0] = ML_NEGINF;
            }
            for i in 0..nb as usize {
                by[i] = by[0];
            }
            return;
        }
        let xna = (nu + 0.5).trunc();
        let mut na = xna as i32;
        if na == 1 {
            nu -= xna;
        }
        if nu == -0.5 {
            let p = M_SQRT_2DPI / ex.sqrt();
            ya = p * ex.sin();
            ya1 = -p * ex.cos();
        } else if ex < 3.0 {
            // Temme's scheme for small X
            let b = ex * 0.5;
            let mut d = -b.ln();
            let mut f = nu * d;
            let mut e = b.powf(-nu);
            let mut c = if nu.abs() < M_EPS_SINC { M_1_PI } else { nu / sinpi(nu) };

            let s = if f.abs() < 1.0 {
                let x2 = f * f;
                let mut en = 19.0;
                let mut ss = 1.0;
                for _ in 1..=9 {
                    ss = ss * x2 / en / (en - 1.0) + 1.0;
                    en -= 2.0;
                }
                ss
            } else {
                (e - 1.0 / e) * 0.5 / f
            };
            // 1/gamma(1-a) by Chebyshev polynomials
            let x2 = nu * nu * 8.0;
            let mut aye = CH[0];
            let mut even = 0.0;
            let mut alfa = CH[1];
            let mut odd = 0.0;
            let mut i = 3;
            while i <= 19 {
                even = -(aye + aye + even);
                aye = -even * x2 - aye + CH[i - 1];
                odd = -(alfa + alfa + odd);
                alfa = -odd * x2 - alfa + CH[i];
                i += 2;
            }
            even = (even * 0.5 + aye) * x2 - aye + CH[20];
            odd = (odd + alfa) * 2.0;
            let gamma = odd * nu + even;
            let mut g = e * gamma;
            e = (e + 1.0 / e) * 0.5;
            f = 2.0 * c * (odd * e + even * s * d);
            e = nu * nu;
            let mut p = g * c;
            let mut q = M_1_PI / g;
            c = nu * M_PI_2;
            let mut r = if c.abs() < M_EPS_SINC { 1.0 } else { sinpi(nu / 2.0) / c };
            r = M_PI * c * r * r;
            c = 1.0;
            d = -b * b;
            let mut h = 0.0;
            ya = f + r * q;
            ya1 = p;
            let mut en = 1.0;
            while (g / (1.0 + ya.abs())).abs() + (h / (1.0 + ya1.abs())).abs() > DBL_EPSILON {
                f = (f * en + p + q) / (en * en - e);
                c *= d / en;
                p /= en - nu;
                q /= en + nu;
                g = c * (f + r * q);
                h = c * p - en * g;
                ya += g;
                ya1 += h;
                en += 1.0;
            }
            ya = -ya;
            ya1 = -ya1 / b;
        } else if ex < THRESH_BESS_Y {
            // Temme's scheme for moderate X: 3 <= x < 16
            let c = (0.5 - nu) * (0.5 + nu);
            let b = ex + ex;
            let mut e = ex * M_1_PI * cospi(nu) / DBL_EPSILON;
            e *= e;
            let mut p = 1.0;
            let mut q = -ex;
            let mut r = 1.0 + ex * ex;
            let mut s = r;
            let mut en = 2.0;
            let mut en1 = 0.0;
            let mut d;
            while r * en * en < e {
                en1 = en + 1.0;
                d = (en - 1.0 + c / en) / s;
                p = (en + en - p * d) / en1;
                q = (-b + q * d) / en1;
                s = p * p + q * q;
                r *= s;
                en = en1;
            }
            let mut f = p / s;
            p = f;
            let mut g = -q / s;
            q = g;
            // L220
            loop {
                en -= 1.0;
                if en > 0.0 {
                    r = en1 * (2.0 - p) - 2.0;
                    s = b + en1 * q;
                    d = (en - 1.0 + c / en) / (r * r + s * s);
                    p = d * r;
                    q = d * s;
                    e = f + 1.0;
                    f = p * e - g * q;
                    g = q * e + p * g;
                    en1 = en;
                } else {
                    break;
                }
            }
            f += 1.0;
            d = f * f + g * g;
            let pa = f / d;
            let qa = -g / d;
            d = nu + 0.5 - p;
            q += ex;
            let pa1 = (pa * q - qa * d) / ex;
            let qa1 = (qa * q + pa * d) / ex;
            let bb = ex - M_PI_2 * (nu + 0.5);
            let cc = bb.cos();
            let ss = bb.sin();
            d = M_SQRT_2DPI / ex.sqrt();
            ya = d * (pa * ss + qa * cc);
            ya1 = d * (qa1 * ss - pa1 * cc);
        } else {
            // Campbell's asymptotic scheme
            na = 0;
            let mut d1 = (ex / FIVPI).trunc();
            let i = d1 as i32;
            let mut dmu = ex - 15.0 * d1 - d1 * PIM5 - (alpha + 0.5) * M_PI_2;
            let (mut cosmu, mut sinmu);
            if i - ((i / 2) << 1) == 0 {
                cosmu = dmu.cos();
                sinmu = dmu.sin();
            } else {
                cosmu = -dmu.cos();
                sinmu = -dmu.sin();
            }
            let ddiv = 8.0 * ex;
            dmu = alpha;
            let den = ex.sqrt();
            ya = 0.0;
            ya1 = 0.0;
            for k in 1..=2 {
                let mut p = cosmu;
                cosmu = sinmu;
                sinmu = -p;
                d1 = (2.0 * dmu - 1.0) * (2.0 * dmu + 1.0);
                let mut d2 = 0.0;
                let mut div = ddiv;
                p = 0.0;
                let mut q = 0.0;
                let q0 = d1 / div;
                let mut term = q0;
                for _ in 2..=20 {
                    d2 += 8.0;
                    d1 -= d2;
                    div += ddiv;
                    term = -term * d1 / div;
                    p += term;
                    d2 += 8.0;
                    d1 -= d2;
                    div += ddiv;
                    term *= d1 / div;
                    q += term;
                    if term.abs() <= DBL_EPSILON {
                        break;
                    }
                }
                p += 1.0;
                q += q0;
                if k == 1 {
                    ya = M_SQRT_2DPI * (p * cosmu - q * sinmu) / den;
                } else {
                    ya1 = M_SQRT_2DPI * (p * cosmu - q * sinmu) / den;
                }
                dmu += 1.0;
            }
        }
        if na == 1 {
            let mut h = 2.0 * (nu + 1.0) / ex;
            if h > 1.0 && ya1.abs() > DBL_MAX / h {
                h = 0.0;
                ya = 0.0;
            }
            h = h * ya1 - ya;
            ya = ya1;
            ya1 = h;
        }
        // Now have first one or two Y's
        by[0] = ya;
        *ncalc = 1;
        'l450: {
            if nb > 1 {
                by[1] = ya1;
                if ya1 != 0.0 {
                    let mut aye = 1.0 + alpha;
                    let twobyx = 2.0 / ex;
                    *ncalc = 2;
                    for i in 2..nb as usize {
                        if twobyx < 1.0 {
                            if by[i - 1].abs() * twobyx >= DBL_MAX / aye {
                                break 'l450;
                            }
                        } else if by[i - 1].abs() >= DBL_MAX / aye / twobyx {
                            break 'l450;
                        }
                        by[i] = twobyx * aye * by[i - 1] - by[i - 2];
                        aye += 1.0;
                        *ncalc += 1;
                    }
                }
            }
        }
        for i in *ncalc..nb {
            by[i as usize] = ML_NEGINF;
        }
    } else {
        by[0] = 0.0;
        *ncalc = min0(nb, 0) - 1;
    }
}

// ----------------------------------------------------------------------
// I

/// R: nmath/bessel_i.c:bessel_i
pub fn bessel_i(x: f64, alpha: f64, expo: f64) -> f64 {
    if x.is_nan() || alpha.is_nan() {
        return x + alpha;
    }
    if x < 0.0 {
        ml_warning(ME_RANGE, "bessel_i");
        return ML_NAN;
    }
    let ize = expo as i32;
    let na = alpha.floor();
    if alpha < 0.0 {
        return bessel_i(x, -alpha, expo)
            + (if alpha == na {
                0.0
            } else {
                bessel_k(x, -alpha, expo) * (if ize == 1 { 2.0 } else { 2.0 * (-2.0 * x).exp() }) / M_PI * sinpi(-alpha)
            });
    }
    let nb = 1 + na as i32;
    let mut bi = vec![0.0; nb as usize];
    bessel_i_core(x, alpha, nb, ize, &mut bi)
}

/// R: nmath/bessel_i.c:bessel_i_ex
pub fn bessel_i_ex(x: f64, alpha: f64, expo: f64, bi: &mut [f64]) -> f64 {
    if x.is_nan() || alpha.is_nan() {
        return x + alpha;
    }
    if x < 0.0 {
        ml_warning(ME_RANGE, "bessel_i");
        return ML_NAN;
    }
    let ize = expo as i32;
    let na = alpha.floor();
    if alpha < 0.0 {
        return bessel_i_ex(x, -alpha, expo, bi)
            + (if alpha == na {
                0.0
            } else {
                bessel_k_ex(x, -alpha, expo, bi) * (if ize == 1 { 2.0 } else { 2.0 * (-2.0 * x).exp() }) / M_PI
                    * sinpi(-alpha)
            });
    }
    let nb = 1 + na as i32;
    bessel_i_core(x, alpha, nb, ize, bi)
}

fn bessel_i_core(x: f64, alpha: f64, nb: i32, ize: i32, bi: &mut [f64]) -> f64 {
    let alpha = alpha - (nb - 1) as f64;
    let mut ncalc = 0;
    i_bessel(x, alpha, nb, ize, bi, &mut ncalc);
    if ncalc != nb {
        if ncalc < 0 {
            mathlib_warning(format!(
                "bessel_i({}): ncalc (={}) != nb (={}); alpha={}. Arg. out of range?\n",
                fmt_g(x),
                ncalc,
                nb,
                fmt_g(alpha)
            ));
        } else {
            mathlib_warning(format!(
                "bessel_i({},nu={}): precision lost in result\n",
                fmt_g(x),
                fmt_g(alpha + nb as f64 - 1.0)
            ));
        }
    }
    bi[(nb - 1) as usize]
}

/// R: nmath/bessel_i.c:I_bessel (C's 1-based bi[] via macro)
fn i_bessel(x: f64, alpha: f64, nb: i32, ize: i32, bo: &mut [f64], ncalc: &mut i32) {
    const CONST: f64 = 1.585;
    macro_rules! bi {
        ($i:expr) => {
            bo[($i) as usize - 1]
        };
    }
    let nu = alpha;
    let twonu = nu + nu;

    if nb > 0 && x >= 0.0 && (0.0..1.0).contains(&nu) && (1..=2).contains(&ize) {
        *ncalc = nb;
        if ize == 1 && x > EXPARG_BESS {
            for k in 1..=nb {
                bi!(k) = ML_POSINF;
            }
            return;
        }
        if ize == 2 && x > XLRG_BESS_IJ {
            for k in 1..=nb {
                bi!(k) = 0.0;
            }
            return;
        }
        let intx = x as i32;
        if x >= RTNSIG_BESS {
            // Initialize the forward sweep, the P-sequence of Olver
            let nbmx = nb - intx;
            let mut n = intx + 1;
            let mut en = (n + n) as f64 + twonu;
            let mut plast = 1.0;
            let mut p = en / x;
            let mut pold;
            let mut test = ENSIG_BESS + ENSIG_BESS;
            if intx << 1 > NSIG_BESS * 5 {
                test = (test * p).sqrt();
            } else {
                test /= r_pow_di(CONST, intx);
            }
            let mut goto_l120 = false;
            if nbmx >= 3 {
                let mut tover = ENTEN_BESS / ENSIG_BESS;
                let mut nstart = intx + 2;
                let mut nend = nb - 1;
                let mut k = nstart;
                while k <= nend {
                    n = k;
                    en += 2.0;
                    pold = plast;
                    plast = p;
                    p = en * plast / x + pold;
                    if p > tover {
                        tover = ENTEN_BESS;
                        p /= tover;
                        plast /= tover;
                        let mut psave = p;
                        let mut psavel = plast;
                        nstart = n + 1;
                        loop {
                            n += 1;
                            en += 2.0;
                            pold = plast;
                            plast = p;
                            p = en * plast / x + pold;
                            if !(p <= 1.0) {
                                break;
                            }
                        }
                        let bb = en / x;
                        test = pold * plast / ENSIG_BESS;
                        test *= 0.5 - 0.5 / (bb * bb);
                        p = plast * tover;
                        n -= 1;
                        en -= 2.0;
                        nend = min0(nb, n);
                        let mut l = nstart;
                        let mut found = false;
                        while l <= nend {
                            *ncalc = l;
                            pold = psavel;
                            psavel = psave;
                            psave = en * psavel / x + pold;
                            if psave * psavel > test {
                                found = true;
                                break;
                            }
                            l += 1;
                        }
                        if !found {
                            *ncalc = nend + 1;
                        }
                        // L90:
                        *ncalc -= 1;
                        goto_l120 = true;
                        break;
                    }
                    k += 1;
                }
                if !goto_l120 {
                    n = nend;
                    en = (n + n) as f64 + twonu;
                    test = fmax2(test, (plast * ENSIG_BESS).sqrt() * (p + p).sqrt());
                }
            }
            if !goto_l120 {
                loop {
                    n += 1;
                    en += 2.0;
                    pold = plast;
                    plast = p;
                    p = en * plast / x + pold;
                    if !(p < test) {
                        break;
                    }
                }
            }
            // L120: backward recursion and normalization sum
            n += 1;
            en += 2.0;
            let mut bb = 0.0;
            let mut aa = 1.0 / p;
            let mut em = n as f64 - 1.0;
            let mut empal = em + nu;
            let mut emp2al = em - 1.0 + twonu;
            let mut sum = aa * empal * emp2al / em;
            let mut nend = n - nb;
            let mut goto_l220 = false;
            let mut goto_l230 = false;
            if nend < 0 {
                bi!(n) = aa;
                nend = -nend;
                for l in 1..=nend {
                    bi!(n + l) = 0.0;
                }
            } else {
                if nend > 0 {
                    for _ in 1..=nend {
                        n -= 1;
                        en -= 2.0;
                        let mut cc = bb;
                        bb = aa;
                        if nend > 100 && aa > 1e200 {
                            cc = ldexp(cc, -900);
                            bb = ldexp(bb, -900);
                            sum = ldexp(sum, -900);
                        }
                        aa = en * bb / x + cc;
                        em -= 1.0;
                        emp2al -= 1.0;
                        if n == 1 {
                            break;
                        }
                        if n == 2 {
                            emp2al = 1.0;
                        }
                        empal -= 1.0;
                        sum = (sum + aa * empal) * emp2al / em;
                    }
                }
                bi!(n) = aa;
                if nb <= 1 {
                    sum = sum + sum + aa;
                    goto_l230 = true;
                } else {
                    n -= 1;
                    en -= 2.0;
                    bi!(n) = en * aa / x + bb;
                    if n == 1 {
                        goto_l220 = true;
                    } else {
                        em -= 1.0;
                        if n == 2 {
                            emp2al = 1.0;
                        } else {
                            emp2al -= 1.0;
                        }
                        empal -= 1.0;
                        sum = (sum + bi!(n) * empal) * emp2al / em;
                    }
                }
            }
            if !goto_l220 && !goto_l230 {
                nend = n - 2;
                if nend > 0 {
                    for _ in 1..=nend {
                        n -= 1;
                        en -= 2.0;
                        bi!(n) = en * bi!(n + 1) / x + bi!(n + 2);
                        em -= 1.0;
                        if n == 2 {
                            emp2al = 1.0;
                        } else {
                            emp2al -= 1.0;
                        }
                        empal -= 1.0;
                        sum = (sum + bi!(n) * empal) * emp2al / em;
                    }
                }
                bi!(1) = 2.0 * empal * bi!(2) / x + bi!(3);
                goto_l220 = true;
            }
            if goto_l220 {
                sum = sum + sum + bi!(1);
            }
            // L230: normalize
            if nu != 0.0 {
                sum *= gamma_cody(1.0 + nu) * (x * 0.5).powf(-nu);
            }
            if ize == 1 {
                sum *= (-x).exp();
            }
            aa = ENMTEN_BESS;
            if sum > 1.0 {
                aa *= sum;
            }
            for n in 1..=nb {
                if bi!(n) < aa {
                    bi!(n) = 0.0;
                } else {
                    bi!(n) /= sum;
                }
            }
        } else {
            // Two-term ascending series for small X
            let mut aa = 1.0;
            let mut empal = 1.0 + nu;
            let halfx = 0.5 * x;
            if nu != 0.0 {
                aa = halfx.powf(nu) / gamma_cody(empal);
            }
            if ize == 2 {
                aa *= (-x).exp();
            }
            let bb = halfx * halfx;
            bi!(1) = aa + aa * bb / empal;
            if x != 0.0 && bi!(1) == 0.0 {
                *ncalc = 0;
            }
            if nb > 1 {
                if x == 0.0 {
                    for n in 2..=nb {
                        bi!(n) = 0.0;
                    }
                } else {
                    let cc = halfx;
                    let mut tover = (ENMTEN_BESS + ENMTEN_BESS) / x;
                    if bb != 0.0 {
                        tover = ENMTEN_BESS / bb;
                    }
                    for n in 2..=nb {
                        aa /= empal;
                        empal += 1.0;
                        aa *= cc;
                        if aa <= tover * empal {
                            aa = 0.0;
                            bi!(n) = 0.0;
                        } else {
                            bi!(n) = aa + aa * bb / empal;
                        }
                        if bi!(n) == 0.0 && *ncalc > n {
                            *ncalc = n - 1;
                        }
                    }
                }
            }
        }
    } else {
        *ncalc = min0(nb, 0) - 1;
    }
}

// ----------------------------------------------------------------------
// K

/// R: nmath/bessel_k.c:bessel_k
pub fn bessel_k(x: f64, alpha: f64, expo: f64) -> f64 {
    if x.is_nan() || alpha.is_nan() {
        return x + alpha;
    }
    if x < 0.0 {
        ml_warning(ME_RANGE, "bessel_k");
        return ML_NAN;
    }
    let alpha = if alpha < 0.0 { -alpha } else { alpha };
    let nb = 1 + alpha.floor() as i32;
    let mut bk = vec![0.0; nb as usize];
    bessel_k_core(x, alpha, nb, expo as i32, &mut bk)
}

/// R: nmath/bessel_k.c:bessel_k_ex
pub fn bessel_k_ex(x: f64, alpha: f64, expo: f64, bk: &mut [f64]) -> f64 {
    if x.is_nan() || alpha.is_nan() {
        return x + alpha;
    }
    if x < 0.0 {
        ml_warning(ME_RANGE, "bessel_k");
        return ML_NAN;
    }
    let alpha = if alpha < 0.0 { -alpha } else { alpha };
    let nb = 1 + alpha.floor() as i32;
    bessel_k_core(x, alpha, nb, expo as i32, bk)
}

fn bessel_k_core(x: f64, alpha: f64, nb: i32, ize: i32, bk: &mut [f64]) -> f64 {
    let alpha = alpha - (nb - 1) as f64;
    let mut ncalc = 0;
    k_bessel(x, alpha, nb, ize, bk, &mut ncalc);
    if ncalc != nb {
        if ncalc < 0 {
            mathlib_warning(format!(
                "bessel_k({}): ncalc (={}) != nb (={}); alpha={}. Arg. out of range?\n",
                fmt_g(x),
                ncalc,
                nb,
                fmt_g(alpha)
            ));
        } else {
            mathlib_warning(format!(
                "bessel_k({},nu={}): precision lost in result\n",
                fmt_g(x),
                fmt_g(alpha + nb as f64 - 1.0)
            ));
        }
    }
    bk[(nb - 1) as usize]
}

/// R: nmath/bessel_k.c:K_bessel (0-based `bk` as in C)
fn k_bessel(x: f64, alpha: f64, nb: i32, ize: i32, bk: &mut [f64], ncalc: &mut i32) {
    const A: f64 = 0.11593151565841244881;
    const P: [f64; 8] = [
        0.805629875690432845,
        20.4045500205365151,
        157.705605106676174,
        536.671116469207504,
        900.382759291288778,
        730.923886650660393,
        229.299301509425145,
        0.822467033424113231,
    ];
    const Q: [f64; 7] = [
        29.4601986247850434,
        277.577868510221208,
        1206.70325591027438,
        2762.91444159791519,
        3443.74050506564618,
        2210.63190113378647,
        572.267338359892221,
    ];
    const R: [f64; 5] = [
        -0.48672575865218401848,
        13.079485869097804016,
        -101.96490580880537526,
        347.65409106507813131,
        3.495898124521934782e-4,
    ];
    const S: [f64; 4] = [-25.579105509976461286, 212.57260432226544008, -610.69018684944109624, 422.69668805777760407];
    const T: [f64; 6] = [
        1.6125990452916363814e-10,
        2.5051878502858255354e-8,
        2.7557319615147964774e-6,
        1.9841269840928373686e-4,
        0.0083333333333334751799,
        0.16666666666666666446,
    ];
    const ESTM: [f64; 6] = [52.0583, 5.7607, 2.7782, 14.4303, 185.3004, 9.3715];
    const ESTF: [f64; 7] = [41.8341, 7.1075, 6.4306, 42.511, 1.35633, 84.5096, 20.];

    let mut ii = 0;
    let ex = x;
    let mut nu = alpha;
    *ncalc = min0(nb, 0) - 2;
    if nb > 0 && (0.0..1.0).contains(&nu) && (1..=2).contains(&ize) {
        if ex <= 0.0 || (ize == 1 && ex > XMAX_BESS_K) {
            if ex <= 0.0 {
                if ex < 0.0 {
                    ml_warning(ME_RANGE, "K_bessel");
                }
                for v in bk.iter_mut().take(nb as usize) {
                    *v = ML_POSINF;
                }
            } else {
                for v in bk.iter_mut().take(nb as usize) {
                    *v = 0.0;
                }
            }
            *ncalc = nb;
            return;
        }
        let mut k = 0;
        if nu < SQXMIN_BESS_K {
            nu = 0.0;
        } else if nu > 0.5 {
            k = 1;
            nu -= 1.0;
        }
        let mut twonu = nu + nu;
        let iend = nb + k - 1;
        let mut c = nu * nu;
        let mut d3 = -c;
        let mut bk1;
        let mut bk2;
        let wminf;
        let mut ratio;
        let goto_l420;
        let mut d1;
        let mut d2;
        let mut t1;
        let mut t2;
        let mut f0;
        let mut f1;
        let mut f2;
        let mut p0;
        let mut q0;
        'compute: {
            if ex <= 1.0 {
                // P0 = GAMMA(1+ALPHA) * (2/X)**ALPHA, Q0 = GAMMA(1-ALPHA) * (X/2)**ALPHA
                d1 = 0.0;
                d2 = P[0];
                t1 = 1.0;
                t2 = Q[0];
                let mut i = 2;
                while i <= 7 {
                    d1 = c * d1 + P[i - 1];
                    d2 = c * d2 + P[i];
                    t1 = c * t1 + Q[i - 1];
                    t2 = c * t2 + Q[i];
                    i += 2;
                }
                d1 *= nu;
                t1 *= nu;
                f1 = ex.ln();
                f0 = A + nu * (P[7] - nu * (d1 + d2) / (t1 + t2)) - f1;
                q0 = (-nu * (A - nu * (P[7] + nu * (d1 - d2) / (t1 - t2)) - f1)).exp();
                f1 = nu * f0;
                p0 = f1.exp();
                // F0
                d1 = R[4];
                t1 = 1.0;
                for i in 0..4 {
                    d1 = c * d1 + R[i];
                    t1 = c * t1 + S[i];
                }
                if f1.abs() <= 0.5 {
                    f1 *= f1;
                    d2 = 0.0;
                    for tv in T.iter() {
                        d2 = f1 * d2 + tv;
                    }
                    d2 = f0 + f0 * f1 * d2;
                } else {
                    d2 = f1.sinh() / nu;
                }
                f0 = d2 - nu * d1 / (t1 * p0);
                if ex <= 1e-10 {
                    // X <= 1.0E-10
                    bk[0] = f0 + ex * f0;
                    if ize == 1 {
                        bk[0] -= ex * bk[0];
                    }
                    ratio = p0 / f0;
                    c = ex * DBL_MAX;
                    if k != 0 {
                        *ncalc = -1;
                        if bk[0] >= c / ratio {
                            return;
                        }
                        bk[0] = ratio * bk[0] / ex;
                        twonu += 2.0;
                        ratio = twonu;
                    }
                    *ncalc = 1;
                    if nb == 1 {
                        return;
                    }
                    *ncalc = -1;
                    for i in 1..nb as usize {
                        if ratio >= c {
                            return;
                        }
                        bk[i] = ratio / ex;
                        twonu += 2.0;
                        ratio = twonu;
                    }
                    *ncalc = 1;
                    goto_l420 = true;
                    break 'compute;
                } else {
                    // 10^-10 < X <= 1.0
                    c = 1.0;
                    let x2by4 = ex * ex / 4.0;
                    p0 *= 0.5;
                    q0 *= 0.5;
                    d1 = -1.0;
                    d2 = 0.0;
                    bk1 = 0.0;
                    bk2 = 0.0;
                    f1 = f0;
                    f2 = p0;
                    loop {
                        d1 += 2.0;
                        d2 += 1.0;
                        d3 += d1;
                        c = x2by4 * c / d2;
                        f0 = (d2 * f0 + p0 + q0) / d3;
                        p0 /= d2 - nu;
                        q0 /= d2 + nu;
                        t1 = c * f0;
                        t2 = c * (p0 - d2 * f0);
                        bk1 += t1;
                        bk2 += t2;
                        if !((t1 / (f1 + bk1)).abs() > DBL_EPSILON || (t2 / (f2 + bk2)).abs() > DBL_EPSILON) {
                            break;
                        }
                    }
                    bk1 += f1;
                    bk2 = 2.0 * (f2 + bk2) / ex;
                    if ize == 2 {
                        d1 = ex.exp();
                        bk1 *= d1;
                        bk2 *= d1;
                    }
                    wminf = ESTF[0] * ex + ESTF[1];
                }
            } else if DBL_EPSILON * ex > 1.0 {
                // X > 1./EPS
                *ncalc = nb;
                bk1 = 1.0 / (M_SQRT_2DPI * ex.sqrt());
                for v in bk.iter_mut().take(nb as usize) {
                    *v = bk1;
                }
                return;
            } else {
                // X > 1.0
                let twox = ex + ex;
                let mut blpha = 0.0;
                ratio = 0.0;
                if ex <= 4.0 {
                    // K(ALPHA+1,X)/K(ALPHA,X), 1.0 <= X <= 4.0
                    d2 = (ESTM[0] / ex + ESTM[1]).trunc();
                    let mut m = d2 as i32;
                    d1 = d2 + d2;
                    d2 -= 0.5;
                    d2 *= d2;
                    for _ in 2..=m {
                        d1 -= 2.0;
                        d2 -= d1;
                        ratio = (d3 + d2) / (twox + d1 - ratio);
                    }
                    // I(|ALPHA|,X) and I(|ALPHA|+1,X) by backward recurrence,
                    // K(ALPHA,X) from the wronskian
                    d2 = (ESTM[2] * ex + ESTM[3]).trunc();
                    m = d2 as i32;
                    c = nu.abs();
                    d3 = c + c;
                    d1 = d3 - 1.0;
                    f1 = DBL_MIN;
                    f0 = (2.0 * (c + d2) / ex + 0.5 * ex / (c + d2 + 1.0)) * DBL_MIN;
                    for _ in 3..=m {
                        d2 -= 1.0;
                        f2 = (d3 + d2 + d2) * f0;
                        blpha = (1.0 + d1 / d2) * (f2 + blpha);
                        f2 = f2 / ex + f1;
                        f1 = f0;
                        f0 = f2;
                    }
                    f1 += (d3 + 2.0) * f0 / ex;
                    d1 = 0.0;
                    t1 = 1.0;
                    for i in 1..=7 {
                        d1 = c * d1 + P[i - 1];
                        t1 = c * t1 + Q[i - 1];
                    }
                    p0 = (c * (A + c * (P[7] - c * d1 / t1) - ex.ln())).exp() / ex;
                    f2 = (c + 0.5 - ratio) * f1 / ex;
                    bk1 = p0 + (d3 * f0 - f2 + f0 + blpha) / (f2 + f1 + f0) * p0;
                    if ize == 1 {
                        bk1 *= (-ex).exp();
                    }
                    wminf = ESTF[2] * ex + ESTF[3];
                } else {
                    // K(ALPHA,X) and K(ALPHA+1,X)/K(ALPHA,X) by backward recurrence, X > 4.0
                    let mut dm = (ESTM[4] / ex + ESTM[5]).trunc();
                    let m = dm as i32;
                    d2 = dm - 0.5;
                    d2 *= d2;
                    d1 = dm + dm;
                    for _ in 2..=m {
                        dm -= 1.0;
                        d1 -= 2.0;
                        d2 -= d1;
                        ratio = (d3 + d2) / (twox + d1 - ratio);
                        blpha = (ratio + ratio * blpha) / dm;
                    }
                    bk1 = 1.0 / ((M_SQRT_2DPI + M_SQRT_2DPI * blpha) * ex.sqrt());
                    if ize == 1 {
                        bk1 *= (-ex).exp();
                    }
                    wminf = ESTF[4] * (ex - (ex - ESTF[6]).abs()) + ESTF[5];
                }
                // K(ALPHA+1,X) from K(ALPHA,X) and K(ALPHA+1,X)/K(ALPHA,X)
                bk2 = bk1 + bk1 * (nu + 0.5 - ratio) / ex;
            }
            // NCALC, K(ALPHA+I,X), I = 0..NCALC-1, and ratios for the rest
            *ncalc = nb;
            bk[0] = bk1;
            if iend == 0 {
                return;
            }
            let mut j = 1 - k;
            if j >= 0 {
                bk[j as usize] = bk2;
            }
            if iend == 1 {
                return;
            }
            let mut m = min0((wminf - nu) as i32, iend);
            for i in 2..=m {
                t1 = bk1;
                bk1 = bk2;
                twonu += 2.0;
                if ex < 1.0 {
                    if bk1 >= DBL_MAX / twonu * ex {
                        break;
                    }
                } else if bk1 / ex >= DBL_MAX / twonu {
                    break;
                }
                bk2 = twonu / ex * bk1 + t1;
                ii = i;
                j += 1;
                if j >= 0 {
                    bk[j as usize] = bk2;
                }
            }
            m = ii;
            if m == iend {
                return;
            }
            ratio = bk2 / bk1;
            let mplus1 = m + 1;
            *ncalc = -1;
            for _ in mplus1..=iend {
                twonu += 2.0;
                ratio = twonu / ex + 1.0 / ratio;
                j += 1;
                if j >= 1 {
                    bk[j as usize] = ratio;
                } else {
                    if bk2 >= DBL_MAX / ratio {
                        return;
                    }
                    bk2 *= ratio;
                }
            }
            *ncalc = max0(1, mplus1 - k);
            if *ncalc == 1 {
                bk[0] = bk2;
            }
            if nb == 1 {
                return;
            }
            goto_l420 = true;
        }
        if goto_l420 {
            let mut i = *ncalc;
            while i < nb {
                bk[i as usize] *= bk[(i - 1) as usize];
                *ncalc += 1;
                i += 1;
            }
        }
    }
}

// ----------------------------------------------------------------------
// The .Internal()s: arithmetic.c math2B (besselJ, besselY) and math3B
// (besselI, besselK). Inputs are already coerced to double; returns the
// values and R's `naflag` (a NaN result from non-NaN inputs, which R
// reports as "NaNs produced").

/// `if_NA_Math2_set` / `if_NA_Math3_set`
fn na_set(xs: &[f64]) -> Option<f64> {
    if xs.iter().any(|&v| is_na(v)) {
        Some(crate::NA_REAL)
    } else if xs.iter().any(|v| v.is_nan()) {
        Some(ML_NAN)
    } else {
        None
    }
}

#[inline]
fn is_na(x: f64) -> bool {
    x.is_nan() && (x.to_bits() & 0xFFFF_FFFF) == 1954
}

fn math2b(a: &[f64], b: &[f64], max_nu: Option<f64>, f: fn(f64, f64, &mut [f64]) -> f64) -> (Vec<f64>, bool) {
    let (na, nb) = (a.len(), b.len());
    if na == 0 || nb == 0 {
        return (vec![], false);
    }
    let n = na.max(nb);
    let mut amax = 0.0f64;
    for &bi in b {
        let av = if bi < 0.0 { -bi } else { bi };
        if amax < av {
            amax = av;
        }
    }
    if let Some(mx) = max_nu {
        if amax > mx {
            amax = mx;
        }
    }
    let nw = 1 + amax.floor() as usize;
    let mut work = vec![0.0f64; nw];
    let mut naflag = false;
    let mut y = Vec::with_capacity(n);
    for i in 0..n {
        let (ai, bi) = (a[i % na], b[i % nb]);
        let v = match na_set(&[ai, bi]) {
            Some(v) => v,
            None => {
                let v = f(ai, bi, &mut work);
                if v.is_nan() {
                    naflag = true;
                }
                v
            }
        };
        y.push(v);
    }
    (y, naflag)
}

fn math3b(a: &[f64], b: &[f64], c: &[f64], f: fn(f64, f64, f64, &mut [f64]) -> f64) -> (Vec<f64>, bool) {
    let (na, nb, nc) = (a.len(), b.len(), c.len());
    if na == 0 || nb == 0 || nc == 0 {
        return (vec![], false);
    }
    let n = na.max(nb).max(nc);
    let mut amax = 0.0f64;
    for &bi in b {
        let av = if bi < 0.0 { -bi } else { bi };
        if av > amax {
            amax = av;
        }
    }
    let nw = 1 + amax.floor() as usize;
    let mut work = vec![0.0f64; nw];
    let mut naflag = false;
    let mut y = Vec::with_capacity(n);
    for i in 0..n {
        let (ai, bi, ci) = (a[i % na], b[i % nb], c[i % nc]);
        let v = match na_set(&[ai, bi, ci]) {
            Some(v) => v,
            None => {
                let v = f(ai, bi, ci, &mut work);
                if v.is_nan() {
                    naflag = true;
                }
                v
            }
        };
        y.push(v);
    }
    (y, naflag)
}

/// `.Internal(besselJ(x, nu))`: arithmetic.c math2B(bessel_j_ex) with the
/// work array sized from max |nu| (capped at 1e7). Returns (values, naflag).
pub fn bessel_j_vec(x: &[f64], nu: &[f64]) -> (Vec<f64>, bool) {
    math2b(x, nu, Some(BESSEL_JY_MAX_NU), bessel_j_ex)
}

/// `.Internal(besselY(x, nu))`: math2B(bessel_y_ex).
pub fn bessel_y_vec(x: &[f64], nu: &[f64]) -> (Vec<f64>, bool) {
    math2b(x, nu, Some(BESSEL_JY_MAX_NU), bessel_y_ex)
}

/// `.Internal(besselI(x, nu, expo))`: math3B(bessel_i_ex); `expo` is
/// 1 (unscaled) or 2 (exponentially scaled).
pub fn bessel_i_vec(x: &[f64], nu: &[f64], expo: &[f64]) -> (Vec<f64>, bool) {
    math3b(x, nu, expo, bessel_i_ex)
}

/// `.Internal(besselK(x, nu, expo))`: math3B(bessel_k_ex).
pub fn bessel_k_vec(x: &[f64], nu: &[f64], expo: &[f64]) -> (Vec<f64>, bool) {
    math3b(x, nu, expo, bessel_k_ex)
}
