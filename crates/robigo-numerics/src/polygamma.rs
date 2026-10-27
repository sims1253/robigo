//! Derivatives of the psi function: polygamma.c (Amos, TOMS 610).

use super::*;

const N_MAX: i32 = 100;

/// d_n(x) = (d/dx)^n cot(x)
fn d_n_cot(x: f64, n: i32) -> f64 {
    match n {
        0 => x.cos() / x.sin(),
        1 => -1.0 / r_pow_di(x.sin(), 2),
        2 => 2.0 * x.cos() / r_pow_di(x.sin(), 3),
        3 => {
            let sin2 = r_pow_di(x.sin(), 2);
            -2.0 * (3.0 - 2.0 * sin2) / r_pow_di(sin2, 2)
        }
        4 => {
            let co = x.cos();
            8.0 * co * (r_pow_di(co, 2) + 2.0) / r_pow_di(x.sin(), 5)
        }
        5 => {
            let co2 = r_pow_di(x.cos(), 2);
            -8.0 * (2.0 * r_pow_di(co2, 2) + 11.0 * co2 + 2.0) / r_pow_di(x.sin(), 6)
        }
        _ => ML_NAN,
    }
}

const BVALUES: [f64; 22] = [
    1.00000000000000000e+00,
    -5.00000000000000000e-01,
    1.66666666666666667e-01,
    -3.33333333333333333e-02,
    2.38095238095238095e-02,
    -3.33333333333333333e-02,
    7.57575757575757576e-02,
    -2.53113553113553114e-01,
    1.16666666666666667e+00,
    -7.09215686274509804e+00,
    5.49711779448621554e+01,
    -5.29124242424242424e+02,
    6.19212318840579710e+03,
    -8.65802531135531136e+04,
    1.42551716666666667e+06,
    -2.72982310678160920e+07,
    6.01580873900642368e+08,
    -1.51163157670921569e+10,
    4.29614643061166667e+11,
    -1.37116552050883328e+13,
    4.88332318973593167e+14,
    -1.92965793419400681e+16,
];

/// R: nmath/polygamma.c:dpsifn. Returns `ierr`; results go to `ans[0..m]`.
pub fn dpsifn(mut x: f64, n: i32, kode: i32, m: i32, ans: &mut [f64], nz: &mut i32) -> i32 {
    *nz = 0;
    if n < 0 || !(1..=2).contains(&kode) || m < 1 {
        return 1;
    }
    if x <= 0.0 {
        if x == c_round(x) {
            for j in 0..m {
                ans[j as usize] = if (j + n) % 2 != 0 { ML_POSINF } else { ML_NAN };
            }
            return 0;
        }
        let ierr = dpsifn(1.0 - x, n, 1, m, ans, nz);
        if n > 5 {
            return 4;
        }
        x *= M_PI;
        let (mut t1, mut t2, mut s) = (1.0, 1.0, 1.0);
        let mut k = 0;
        let mut j = k - n;
        while j < m {
            t1 *= M_PI;
            if k >= 2 {
                t2 *= k as f64;
            }
            if j >= 0 {
                ans[j as usize] = s * (ans[j as usize] + t1 / t2 * d_n_cot(x, k));
            }
            k += 1;
            j += 1;
            s = -s;
        }
        return ierr;
    }

    let xln = x.ln();
    if kode == 1 && m == 1 {
        let lrg = 1.0 / (2.0 * DBL_EPSILON);
        if n == 0 && x * xln > lrg {
            ans[0] = -xln;
            return 0;
        } else if n >= 1 && x > n as f64 * lrg {
            ans[0] = (-(n as f64) * xln).exp() / n as f64;
            return 0;
        }
    }
    let mut nx = imin2(-DBL_MIN_EXP, DBL_MAX_EXP);
    let r1m5 = M_LOG10_2;
    let r1m4 = DBL_EPSILON * 0.5;
    let wdtol = fmax2(r1m4, 0.5e-18);

    let elim = 2.302 * (nx as f64 * r1m5 - 3.0);
    let rln = fmin2(r1m5 * DBL_MANT_DIG as f64, 18.06);
    let mut fln = fmax2(rln, 3.0) - 3.0;
    let yint = 3.50 + 0.40 * fln;
    let slope = 0.21 + fln * (0.0006038 * fln + 0.008677);
    let mut mm = m;
    let mut trm = [0.0f64; 23];
    let mut trmr = [0.0f64; (N_MAX + 1) as usize];

    let (mut nn, mut fn_, mut t);
    let (mut xdmy, mut xdmln, mut xinc) = (0.0, 0.0, 0.0);
    let mut xmin;
    let mut xm;
    let mut goto_l10 = false;
    loop {
        nn = n + mm - 1;
        fn_ = nn as f64;
        t = (fn_ + 1.0) * xln;

        if t.abs() > elim {
            if t <= 0.0 {
                return 2;
            }
        } else {
            if x < wdtol {
                ans[0] = r_pow_di(x, -n - 1);
                if mm != 1 {
                    for k in 1..mm as usize {
                        ans[k] = ans[k - 1] / x;
                    }
                }
                if n == 0 && kode == 2 {
                    ans[0] += xln;
                }
                return 0;
            }
            xm = yint + slope * fn_;
            let mx = xm as i32 + 1;
            xmin = mx as f64;
            if n != 0 {
                xm = -2.302 * rln - fmin2(0.0, xln);
                let arg = fmin2(0.0, xm / n as f64);
                let eps = arg.exp();
                xm = if arg.abs() < 1.0e-3 { -arg } else { 1.0 - eps };
                fln = x * xm / eps;
                xm = xmin - x;
                if xm > 7.0 && fln < 15.0 {
                    break;
                }
            }
            xdmy = x;
            xdmln = xln;
            xinc = 0.0;
            if x < xmin {
                nx = x as i32;
                xinc = xmin - nx as f64;
                xdmy = x + xinc;
                xdmln = xdmy.ln();
            }
            t = fn_ * xdmln;
            let t1 = xdmln + xdmln;
            let t2 = t + xdmln;
            let tk = fmax2(t.abs(), fmax2(t1.abs(), t2.abs()));
            if tk <= elim {
                goto_l10 = true;
                break;
            }
        }
        // underflow ("nz++" in C increments the pointer, not the count)
        mm -= 1;
        ans[mm as usize] = 0.0;
        if mm == 0 {
            return 0;
        }
    }

    if !goto_l10 {
        // series for large n
        let nn = fln as i32 + 1;
        let np = n + 1;
        let t1 = (n + 1) as f64 * xln;
        let mut t = (-t1).exp();
        let mut s = t;
        let mut den = x;
        for i in 1..=nn as usize {
            den += 1.0;
            trm[i] = den.powf(-(np as f64));
            s += trm[i];
        }
        ans[0] = s;
        if n == 0 && kode == 2 {
            ans[0] = s + xln;
        }
        if mm != 1 {
            let tol = wdtol / 5.0;
            for j in 1..mm as usize {
                t /= x;
                s = t;
                let tols = t * tol;
                den = x;
                for i in 1..=nn as usize {
                    den += 1.0;
                    trm[i] /= den;
                    s += trm[i];
                    if trm[i] < tols {
                        break;
                    }
                }
                ans[j] = s;
            }
        }
        return 0;
    }

    // L10:
    let mut tss = (-t).exp();
    let tt = 0.5 / xdmy;
    let mut t1 = tt;
    let tst = wdtol * tt;
    if nn != 0 {
        t1 = tt + 1.0 / fn_;
    }
    let rxsq = 1.0 / (xdmy * xdmy);
    let ta = 0.5 * rxsq;
    t = (fn_ + 1.0) * ta;
    let mut s = t * BVALUES[2];
    if s.abs() >= tst {
        let mut tk = 2.0;
        for k in 4..=22usize {
            t = t * ((tk + fn_ + 1.0) / (tk + 1.0)) * ((tk + fn_) / (tk + 2.0)) * rxsq;
            trm[k] = t * BVALUES[k - 1];
            if trm[k].abs() < tst {
                break;
            }
            s += trm[k];
            tk += 2.0;
        }
    }
    s = (s + t1) * tss;
    let mut goto_l20 = false;
    if xinc != 0.0 {
        nx = xinc as i32;
        let np = nn + 1;
        if nx > N_MAX {
            return 3;
        } else if nn == 0 {
            goto_l20 = true;
        } else {
            let mut xm = xinc - 1.0;
            let mut fx = x + xm;
            for i in 1..=nx as usize {
                trmr[i] = fx.powf(-(np as f64));
                s += trmr[i];
                xm -= 1.0;
                fx = x + xm;
            }
        }
    }
    if !goto_l20 {
        ans[(mm - 1) as usize] = s;
        if fn_ != 0.0 {
            // generate lower derivatives, j < n+mm-1
            let mut finished = false;
            for j in 2..=mm {
                fn_ -= 1.0;
                tss *= xdmy;
                t1 = tt;
                if fn_ != 0.0 {
                    t1 = tt + 1.0 / fn_;
                }
                t = (fn_ + 1.0) * ta;
                s = t * BVALUES[2];
                if s.abs() >= tst {
                    let mut tk = 4.0 + fn_;
                    for k in 4..=22usize {
                        trm[k] = trm[k] * (fn_ + 1.0) / tk;
                        if trm[k].abs() < tst {
                            break;
                        }
                        s += trm[k];
                        tk += 2.0;
                    }
                }
                s = (s + t1) * tss;
                if xinc != 0.0 {
                    if fn_ == 0.0 {
                        goto_l20 = true;
                        finished = true;
                        break;
                    }
                    let mut xm = xinc - 1.0;
                    let mut fx = x + xm;
                    for i in 1..=nx as usize {
                        trmr[i] *= fx;
                        s += trmr[i];
                        xm -= 1.0;
                        fx = x + xm;
                    }
                }
                ans[(mm - j) as usize] = s;
                if fn_ == 0.0 {
                    finished = true;
                    break;
                }
            }
            if !finished {
                return 0;
            }
        }
    }
    if goto_l20 {
        for i in 1..=nx {
            s += 1.0 / (x + (nx - i) as f64);
        }
    }
    // L30:
    if kode != 2 {
        ans[0] = s - xdmln;
    } else if xdmy != x {
        let xq = xdmy / x;
        ans[0] = s - xq.ln();
    }
    0
}

/// R: nmath/polygamma.c:psigamma
pub fn psigamma(x: f64, deriv: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let deriv = r_forceint(deriv);
    let n = deriv as i32;
    if n > N_MAX {
        mathlib_warning(format!("deriv = {} > {} (= n_max)\n", n, N_MAX));
        return ML_NAN;
    }
    let mut ans = [0.0];
    let mut nz = 0;
    if dpsifn(x, n, 1, 1, &mut ans, &mut nz) != 0 {
        return ML_NAN;
    }
    let mut a = -ans[0];
    for k in 1..=n {
        a *= -(k as f64);
    }
    a
}

/// R: nmath/polygamma.c:digamma
pub fn digamma(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let mut ans = [0.0];
    let mut nz = 0;
    if dpsifn(x, 0, 1, 1, &mut ans, &mut nz) != 0 {
        return ML_NAN;
    }
    -ans[0]
}

/// R: nmath/polygamma.c:trigamma
pub fn trigamma(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let mut ans = [0.0];
    let mut nz = 0;
    if dpsifn(x, 1, 1, 1, &mut ans, &mut nz) != 0 {
        return ML_NAN;
    }
    ans[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psi_values() {
        // GNU R 4.5.0, sprintf("%.17g", ...)
        assert_eq!(digamma(1.0), -0.57721566490153231);
        assert_eq!(digamma(0.25), -4.2274535333762655);
        assert_eq!(digamma(-1.5), 0.70315664064524397);
        assert_eq!(digamma(100.0), 4.6001618527380881);
        assert_eq!(trigamma(1.0), 1.6449340668482264);
        assert_eq!(trigamma(-0.5), 8.934802200544679);
        assert_eq!(psigamma(2.5, 3.0), 0.22390584881725206);
        assert!(digamma(0.0).is_nan());
        assert_eq!(trigamma(0.0), f64::INFINITY);
        assert_eq!(psigamma(-2.3, 2.0), 68.713792529270421);
        assert_eq!(psigamma(1e-5, 4.0), -2.4000000000000066e+26);
        assert_eq!(digamma(1e-300), -9.999999999999999e+299);
        assert_eq!(trigamma(3e8), 3.3333333388888906e-09);
        assert_eq!(psigamma(0.3, 7.0), 76818182.998493209);
    }
}
