//! The gamma function and relatives: gamma.c, lgamma.c, lgammacor.c,
//! chebyshev.c, stirlerr.c, beta.c, lbeta.c, choose.c.

use super::*;

/// R: nmath/chebyshev.c:chebyshev_eval
pub fn chebyshev_eval(x: f64, a: &[f64], n: usize) -> f64 {
    if !(1..=1000).contains(&n) {
        return ML_NAN;
    }
    if !(-1.1..=1.1).contains(&x) {
        return ML_NAN;
    }
    let twox = x * 2.0;
    let (mut b0, mut b1, mut b2) = (0.0, 0.0, 0.0);
    for i in 1..=n {
        b2 = b1;
        b1 = b0;
        b0 = twox * b1 - b2 + a[n - i];
    }
    (b0 - b2) * 0.5
}

const GAMCS: [f64; 42] = [
    0.8571195590989331421920062399942e-2,
    0.4415381324841006757191315771652e-2,
    0.5685043681599363378632664588789e-1,
    -0.4219835396418560501012500186624e-2,
    0.1326808181212460220584006796352e-2,
    -0.1893024529798880432523947023886e-3,
    0.3606925327441245256578082217225e-4,
    -0.6056761904460864218485548290365e-5,
    0.1055829546302283344731823509093e-5,
    -0.1811967365542384048291855891166e-6,
    0.3117724964715322277790254593169e-7,
    -0.5354219639019687140874081024347e-8,
    0.9193275519859588946887786825940e-9,
    -0.1577941280288339761767423273953e-9,
    0.2707980622934954543266540433089e-10,
    -0.4646818653825730144081661058933e-11,
    0.7973350192007419656460767175359e-12,
    -0.1368078209830916025799499172309e-12,
    0.2347319486563800657233471771688e-13,
    -0.4027432614949066932766570534699e-14,
    0.6910051747372100912138336975257e-15,
    -0.1185584500221992907052387126192e-15,
    0.2034148542496373955201026051932e-16,
    -0.3490054341717405849274012949108e-17,
    0.5987993856485305567135051066026e-18,
    -0.1027378057872228074490069778431e-18,
    0.1762702816060529824942759660748e-19,
    -0.3024320653735306260958772112042e-20,
    0.5188914660218397839717833550506e-21,
    -0.8902770842456576692449251601066e-22,
    0.1527474068493342602274596891306e-22,
    -0.2620731256187362900257328332799e-23,
    0.4496464047830538670331046570666e-24,
    -0.7714712731336877911703901525333e-25,
    0.1323635453126044036486572714666e-25,
    -0.2270999412942928816702313813333e-26,
    0.3896418998003991449320816639999e-27,
    -0.6685198115125953327792127999999e-28,
    0.1146998663140024384347613866666e-28,
    -0.1967938586345134677295103999999e-29,
    0.3376448816585338090334890666666e-30,
    -0.5793070335782135784625493333333e-31,
];

/// R: nmath/gamma.c:gammafn
pub fn gammafn(x: f64) -> f64 {
    const NGAM: usize = 22;
    const XMIN: f64 = -170.5674972726612;
    const XMAX: f64 = 171.61447887182298;
    const XSML: f64 = 2.2474362225598545e-308;
    const DXREL: f64 = 1.490116119384765696e-8;

    if x.is_nan() {
        return x;
    }
    if x == 0.0 || (x < 0.0 && x == c_round(x)) {
        ml_warning(ME_DOMAIN, "gammafn");
        return ML_NAN;
    }

    let mut y = x.abs();
    let mut value;
    if y <= 10.0 {
        let mut n = x as i32;
        if x < 0.0 {
            n -= 1;
        }
        y = x - n as f64;
        n -= 1;
        value = chebyshev_eval(y * 2.0 - 1.0, &GAMCS, NGAM) + 0.9375;
        if n == 0 {
            return value;
        }
        if n < 0 {
            if x < -0.5 && (x - ((x - 0.5) as i32) as f64 / x).abs() < DXREL {
                ml_warning(ME_PRECISION, "gammafn");
            }
            if y < XSML {
                ml_warning(ME_RANGE, "gammafn");
                return if x > 0.0 { ML_POSINF } else { ML_NEGINF };
            }
            let n = -n;
            for i in 0..n {
                value /= x + i as f64;
            }
            value
        } else {
            for i in 1..=n {
                value *= y + i as f64;
            }
            value
        }
    } else {
        if x > XMAX {
            return ML_POSINF;
        }
        if x < XMIN {
            return 0.0;
        }
        if y <= 50.0 && y == (y as i32) as f64 {
            value = 1.0;
            let mut i = 2;
            while (i as f64) < y {
                value *= i as f64;
                i += 1;
            }
        } else {
            // (2*y == (int)2*y): note the C cast binds to 2 only.
            let corr = if 2.0 * y == ((2i32) as f64) * y { stirlerr(y) } else { lgammacor(y) };
            value = ((y - 0.5) * y.ln() - y + M_LN_SQRT_2PI + corr).exp();
        }
        if x > 0.0 {
            return value;
        }
        if ((x - ((x - 0.5) as i32) as f64) / x).abs() < DXREL {
            ml_warning(ME_PRECISION, "gammafn");
        }
        let sinpiy = sinpi(y);
        if sinpiy == 0.0 {
            ml_warning(ME_RANGE, "gammafn");
            return ML_POSINF;
        }
        -M_PI / (y * sinpiy * value)
    }
}

/// R: nmath/lgamma.c:lgammafn_sign
pub fn lgammafn_sign(x: f64, sgn: Option<&mut i32>) -> f64 {
    const XMAX: f64 = 2.5327372760800758e+305;
    const DXREL: f64 = 1.490116119384765625e-8;

    let mut sgn = sgn;
    if let Some(s) = sgn.as_deref_mut() {
        *s = 1;
    }
    if x.is_nan() {
        return x;
    }
    if let Some(s) = sgn.as_deref_mut() {
        if x < 0.0 && fmod((-x).floor(), 2.0) == 0.0 {
            *s = -1;
        }
    }
    if x <= 0.0 && x == x.trunc() {
        return ML_POSINF;
    }
    let y = x.abs();
    if y < 1e-306 {
        return -y.ln();
    }
    if y <= 10.0 {
        return gammafn(x).abs().ln();
    }
    if y > XMAX {
        return ML_POSINF;
    }
    if x > 0.0 {
        if x > 1e17 {
            return x * (x.ln() - 1.0);
        } else if x > 4934720.0 {
            return M_LN_SQRT_2PI + (x - 0.5) * x.ln() - x;
        } else {
            return M_LN_SQRT_2PI + (x - 0.5) * x.ln() - x + lgammacor(x);
        }
    }
    let sinpiy = sinpi(y).abs();
    if sinpiy == 0.0 {
        mathlib_warning(format!(" ** should NEVER happen! *** [lgamma.c: Neg.int, y={}]\n", fmt_g(y)));
        return ML_NAN;
    }
    let ans = M_LN_SQRT_PID2 + (x - 0.5) * y.ln() - x - sinpiy.ln() - lgammacor(y);
    if ((x - (x - 0.5).trunc()) * ans / x).abs() < DXREL {
        ml_warning(ME_PRECISION, "lgamma");
    }
    ans
}

/// R: nmath/lgamma.c:lgammafn
pub fn lgammafn(x: f64) -> f64 {
    lgammafn_sign(x, None)
}

/// R: nmath/lgammacor.c:lgammacor
pub fn lgammacor(x: f64) -> f64 {
    const ALGMCS: [f64; 15] = [
        0.1666389480451863247205729650822e+0,
        -0.1384948176067563840732986059135e-4,
        0.9810825646924729426157171547487e-8,
        -0.1809129475572494194263306266719e-10,
        0.6221098041892605227126015543416e-13,
        -0.3399615005417721944303330599666e-15,
        0.2683181998482698748957538846666e-17,
        -0.2868042435334643284144622399999e-19,
        0.3962837061046434803679306666666e-21,
        -0.6831888753985766870111999999999e-23,
        0.1429227355942498147573333333333e-24,
        -0.3547598158101070547199999999999e-26,
        0.1025680058010470912000000000000e-27,
        -0.3401102254316748799999999999999e-29,
        0.1276642195630062933333333333333e-30,
    ];
    const NALGM: usize = 5;
    const XBIG: f64 = 94906265.62425156;

    if x < 10.0 {
        return ML_NAN;
    } else if x < XBIG {
        let tmp = 10.0 / x;
        return chebyshev_eval(tmp * tmp * 2.0 - 1.0, &ALGMCS, NALGM) / x;
    }
    1.0 / (x * 12.0)
}

/// R: nmath/stirlerr.c:stirlerr
pub fn stirlerr(n: f64) -> f64 {
    const S0: f64 = 0.083333333333333333333;
    const S1: f64 = 0.00277777777777777777778;
    const S2: f64 = 0.00079365079365079365079365;
    const S3: f64 = 0.000595238095238095238095238;
    const S4: f64 = 0.0008417508417508417508417508;
    const S5: f64 = 0.0019175269175269175269175262;
    const S6: f64 = 0.0064102564102564102564102561;
    const S7: f64 = 0.029550653594771241830065352;
    const S8: f64 = 0.17964437236883057316493850;
    const S9: f64 = 1.3924322169059011164274315;
    const S10: f64 = 13.402864044168391994478957;
    const S11: f64 = 156.84828462600201730636509;
    const S12: f64 = 2193.1033333333333333333333;
    const S13: f64 = 36108.771253724989357173269;
    const S14: f64 = 691472.26885131306710839498;
    const S15: f64 = 15238221.539407416192283370;
    const S16: f64 = 382900751.39141414141414141;

    const SFERR_HALVES: [f64; 31] = [
        0.0,
        0.1534264097200273452913848,
        0.0810614667953272582196702,
        0.0548141210519176538961390,
        0.0413406959554092940938221,
        0.03316287351993628748511048,
        0.02767792568499833914878929,
        0.02374616365629749597132920,
        0.02079067210376509311152277,
        0.01848845053267318523077934,
        0.01664469118982119216319487,
        0.01513497322191737887351255,
        0.01387612882307074799874573,
        0.01281046524292022692424986,
        0.01189670994589177009505572,
        0.01110455975820691732662991,
        0.010411265261972096497478567,
        0.009799416126158803298389475,
        0.009255462182712732917728637,
        0.008768700134139385462952823,
        0.008330563433362871256469318,
        0.007934114564314020547248100,
        0.007573675487951840794972024,
        0.007244554301320383179543912,
        0.006942840107209529865664152,
        0.006665247032707682442354394,
        0.006408994188004207068439631,
        0.006171712263039457647532867,
        0.005951370112758847735624416,
        0.005746216513010115682023589,
        0.005554733551962801371038690,
    ];

    if n <= 23.5 {
        let nn = n + n;
        if n <= 15.0 && nn == (nn as i32) as f64 {
            return SFERR_HALVES[nn as i32 as usize];
        }
        if n <= 5.25 {
            if n >= 1.0 {
                let l_n = n.ln();
                return c_lgamma(n) + n * (1.0 - l_n) + ldexp(l_n - M_LN_2PI, -1);
            } else {
                return super::pgamma::lgamma1p(n) - (n + 0.5) * n.ln() + n - M_LN_SQRT_2PI;
            }
        }
        let nn = n * n;
        if n > 12.8 {
            return (S0 - (S1 - (S2 - (S3 - (S4 - (S5 - S6 / nn) / nn) / nn) / nn) / nn) / nn) / n;
        }
        if n > 12.3 {
            return (S0 - (S1 - (S2 - (S3 - (S4 - (S5 - (S6 - S7 / nn) / nn) / nn) / nn) / nn) / nn) / nn) / n;
        }
        if n > 8.9 {
            return (S0 - (S1 - (S2 - (S3 - (S4 - (S5 - (S6 - (S7 - S8 / nn) / nn) / nn) / nn) / nn) / nn) / nn) / nn)
                / n;
        }
        if n > 7.3 {
            return (S0
                - (S1
                    - (S2 - (S3 - (S4 - (S5 - (S6 - (S7 - (S8 - (S9 - S10 / nn) / nn) / nn) / nn) / nn) / nn) / nn) / nn)
                        / nn)
                    / nn)
                / n;
        }
        if n > 6.6 {
            return (S0
                - (S1
                    - (S2
                        - (S3
                            - (S4
                                - (S5 - (S6 - (S7 - (S8 - (S9 - (S10 - (S11 - S12 / nn) / nn) / nn) / nn) / nn) / nn) / nn)
                                    / nn)
                                / nn)
                            / nn)
                        / nn)
                    / nn)
                / n;
        }
        if n > 6.1 {
            return (S0
                - (S1
                    - (S2
                        - (S3
                            - (S4
                                - (S5
                                    - (S6
                                        - (S7
                                            - (S8 - (S9 - (S10 - (S11 - (S12 - (S13 - S14 / nn) / nn) / nn) / nn) / nn) / nn)
                                                / nn)
                                            / nn)
                                        / nn)
                                    / nn)
                                / nn)
                            / nn)
                        / nn)
                    / nn)
                / n;
        }
        return (S0
            - (S1
                - (S2
                    - (S3
                        - (S4
                            - (S5
                                - (S6
                                    - (S7
                                        - (S8
                                            - (S9
                                                - (S10
                                                    - (S11 - (S12 - (S13 - (S14 - (S15 - S16 / nn) / nn) / nn) / nn) / nn)
                                                        / nn)
                                                    / nn)
                                                / nn)
                                            / nn)
                                        / nn)
                                    / nn)
                                / nn)
                            / nn)
                        / nn)
                    / nn)
                / nn)
            / n;
    }
    let nn = n * n;
    if n > 15.7e6 {
        return S0 / n;
    }
    if n > 6180.0 {
        return (S0 - S1 / nn) / n;
    }
    if n > 205.0 {
        return (S0 - (S1 - S2 / nn) / nn) / n;
    }
    if n > 86.0 {
        return (S0 - (S1 - (S2 - S3 / nn) / nn) / nn) / n;
    }
    if n > 27.0 {
        return (S0 - (S1 - (S2 - (S3 - S4 / nn) / nn) / nn) / nn) / n;
    }
    (S0 - (S1 - (S2 - (S3 - (S4 - S5 / nn) / nn) / nn) / nn) / nn) / n
}

/// R: nmath/beta.c:beta
pub fn beta(a: f64, b: f64) -> f64 {
    const XMAX: f64 = 171.61447887182298;
    if a.is_nan() || b.is_nan() {
        return a + b;
    }
    if a < 0.0 || b < 0.0 {
        return ML_NAN;
    } else if a == 0.0 || b == 0.0 {
        return ML_POSINF;
    } else if !a.is_finite() || !b.is_finite() {
        return 0.0;
    }
    if a + b < XMAX {
        (1.0 / gammafn(a + b)) * gammafn(a) * gammafn(b)
    } else {
        lbeta(a, b).exp()
    }
}

/// R: nmath/lbeta.c:lbeta
pub fn lbeta(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return a + b;
    }
    let (mut p, mut q) = (a, a);
    if b < p {
        p = b;
    }
    if b > q {
        q = b;
    }
    if p < 0.0 {
        return ML_NAN;
    } else if p == 0.0 {
        return ML_POSINF;
    } else if !q.is_finite() {
        return ML_NEGINF;
    }
    if p >= 10.0 {
        let corr = lgammacor(p) + lgammacor(q) - lgammacor(p + q);
        q.ln() * -0.5 + M_LN_SQRT_2PI + corr + (p - 0.5) * (p / (p + q)).ln() + q * log1p(-p / (p + q))
    } else if q >= 10.0 {
        let corr = lgammacor(q) - lgammacor(p + q);
        lgammafn(p) + corr + p - p * (p + q).ln() + (q - 0.5) * log1p(-p / (p + q))
    } else if p < 1e-306 {
        c_lgamma(p) + (c_lgamma(q) - c_lgamma(p + q))
    } else {
        (gammafn(p) * (gammafn(q) / gammafn(p + q))).ln()
    }
}

/// R: nmath/choose.c:lfastchoose
pub fn lfastchoose(n: f64, k: f64) -> f64 {
    -(n + 1.0).ln() - lbeta(n - k + 1.0, k + 1.0)
}

fn lfastchoose2(n: f64, k: f64, s_choose: &mut i32) -> f64 {
    let r = lgammafn_sign(n - k + 1.0, Some(s_choose));
    lgammafn(n + 1.0) - lgammafn(k + 1.0) - r
}

fn odd(k: f64) -> bool {
    k != 2.0 * (k / 2.0).floor()
}

fn non_int_warn_rounding(k: f64, k0: f64) {
    if (k - k0).abs() > 1e-9 * fmax2(1.0, k0.abs()) {
        mathlib_warning(format!("'k' ({:.2}) must be integer, rounded to {:.0}", k0, k));
    }
}

/// R: nmath/choose.c:lchoose
pub fn lchoose(n: f64, k: f64) -> f64 {
    let k0 = k;
    let k = r_forceint(k);
    if n.is_nan() || k.is_nan() {
        return n + k;
    }
    non_int_warn_rounding(k, k0);
    if k < 2.0 {
        if k < 0.0 {
            return ML_NEGINF;
        }
        if k == 0.0 {
            return 0.0;
        }
        return n.abs().ln();
    }
    if n < 0.0 {
        return lchoose(-n + k - 1.0, k);
    } else if !r_nonint(n) {
        let n = r_forceint(n);
        if n < k {
            return ML_NEGINF;
        }
        if n - k < 2.0 {
            return lchoose(n, n - k);
        }
        return lfastchoose(n, k);
    }
    if n < k - 1.0 {
        let mut s = 0;
        return lfastchoose2(n, k, &mut s);
    }
    lfastchoose(n, k)
}

/// R: nmath/choose.c:choose
pub fn choose(n: f64, k: f64) -> f64 {
    const K_SMALL_MAX: f64 = 30.0;
    let k0 = k;
    let mut k = r_forceint(k);
    if n.is_nan() || k.is_nan() {
        return n + k;
    }
    non_int_warn_rounding(k, k0);
    if k < K_SMALL_MAX {
        if n - k < k && n >= 0.0 && !r_nonint(n) {
            k = r_forceint(n - k);
        }
        if k < 0.0 {
            return 0.0;
        }
        if k == 0.0 {
            return 1.0;
        }
        let mut r = n;
        let mut j = 2;
        while j as f64 <= k {
            r *= (n - j as f64 + 1.0) / j as f64;
            j += 1;
        }
        return if !r_nonint(n) { r_forceint(r) } else { r };
    }
    if n < 0.0 {
        let mut r = choose(-n + k - 1.0, k);
        if odd(k) {
            r = -r;
        }
        return r;
    } else if !r_nonint(n) {
        let n = r_forceint(n);
        if n < k {
            return 0.0;
        }
        if n - k < K_SMALL_MAX {
            return choose(n, n - k);
        }
        return r_forceint(lfastchoose(n, k).exp());
    }
    if n < k - 1.0 {
        let mut s_choose = 0;
        let r = lfastchoose2(n, k, &mut s_choose);
        return s_choose as f64 * r.exp();
    }
    lfastchoose(n, k).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        a == b || ((a - b) / b).abs() < 4.0 * f64::EPSILON
    }

    // Reference values from GNU R 4.5.0: sprintf("%.17g", ...)
    #[test]
    fn gamma_values() {
        assert_eq!(gammafn(5.0), 24.0);
        assert_eq!(gammafn(0.5), 1.7724538509055161);
        assert_eq!(gammafn(-1.5), 2.3632718012073548);
        assert_eq!(gammafn(10.5), 1133278.3889487833);
        assert_eq!(gammafn(171.5), 9.4833675668258129e+307);
        assert!(gammafn(0.0).is_nan());
        assert!(gammafn(-2.0).is_nan());
        assert_eq!(lgammafn(100.0), 359.1342053695754);
        assert_eq!(lgammafn(0.5), 0.57236494292470008);
        assert_eq!(lgammafn(-10.5), -15.147270590717842);
        assert_eq!(lgammafn(1e10), 220258509288.81058);
    }

    #[test]
    fn beta_choose_values() {
        assert!(close(beta(2.0, 3.0), 0.083333333333333329));
        assert_eq!(lbeta(100.0, 200.0), -192.13419227497894);
        assert_eq!(choose(5.0, 2.0), 10.0);
        assert_eq!(choose(50.0, 25.0), 126410606437752.0);
        assert_eq!(choose(-3.0, 2.0), 6.0);
        assert_eq!(choose(0.5, 3.0), 0.0625);
        assert_eq!(lchoose(50.0, 25.0), 32.470556505811984);
    }
}
