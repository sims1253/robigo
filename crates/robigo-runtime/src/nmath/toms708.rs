//! ACM TOMS 708: the incomplete beta function ratio I_x(a,b), with R's
//! log-scale extensions (nmath/toms708.c).

#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::*;

/// log(1 - exp(x)); toms708.c redefines R_Log1_Exp in terms of rexpm1().
#[inline]
fn r_log1_exp(x: f64) -> f64 {
    if x > -M_LN2 { (-rexpm1(x)).ln() } else { log1p(-x.exp()) }
}

#[inline]
fn min(a: f64, b: f64) -> f64 {
    if a < b { a } else { b }
}

#[inline]
fn max(a: f64, b: f64) -> f64 {
    if a > b { a } else { b }
}

/// Rf_d1mach(3) = DBL_EPSILON / 2
const D1MACH3: f64 = DBL_EPSILON * 0.5;

#[derive(Clone, Copy)]
enum Label {
    WBpser,
    W1Bpser,
    Bfrac,
    L140,
    L131,
    EndFromW,
    EndFromW1,
    EndFromW1Log,
    End,
}

/// R: nmath/toms708.c:bratio. Returns `(w, w1, ierr)` with
/// w = I_x(a,b) and w1 = 1 - I_x(a,b) (both on log scale when `log_p`).
pub fn bratio(a: f64, b: f64, x: f64, y: f64, log_p: bool) -> (f64, f64, i32) {
    let mut eps = 2.0 * D1MACH3;
    let mut w = r_d__0(log_p);
    let mut w1 = r_d__0(log_p);

    if x.is_nan() || y.is_nan() || a.is_nan() || b.is_nan() {
        return (w, w1, 9);
    }
    if a < 0.0 || b < 0.0 {
        return (w, w1, 1);
    }
    if a == 0.0 && b == 0.0 {
        return (w, w1, 2);
    }
    if !(0.0..=1.0).contains(&x) {
        return (w, w1, 3);
    }
    if !(0.0..=1.0).contains(&y) {
        return (w, w1, 4);
    }
    let z = x + y - 0.5 - 0.5;
    if z.abs() > eps * 3.0 {
        return (w, w1, 5);
    }
    let mut ierr = 0;
    if x == 0.0 {
        // L200
        if a == 0.0 {
            return (w, w1, 6);
        }
        return (r_d__0(log_p), r_d__1(log_p), ierr);
    }
    if y == 0.0 {
        // L210
        if b == 0.0 {
            return (w, w1, 7);
        }
        return (r_d__1(log_p), r_d__0(log_p), ierr);
    }
    if a == 0.0 {
        // L211
        return (r_d__1(log_p), r_d__0(log_p), ierr);
    }
    if b == 0.0 {
        // L201
        return (r_d__0(log_p), r_d__1(log_p), ierr);
    }

    eps = max(eps, 1e-15);
    let a_lt_b = a < b;
    if (if a_lt_b { b } else { a }) < eps * 0.001 {
        if log_p {
            if a_lt_b {
                w = log1p(-a / (a + b));
                w1 = (a / (a + b)).ln();
            } else {
                w = (b / (a + b)).ln();
                w1 = log1p(-b / (a + b));
            }
        } else {
            w = b / (a + b);
            w1 = a / (a + b);
        }
        return (w, w1, ierr);
    }

    let do_swap;
    let (mut a0, mut b0, x0, y0);
    let mut lambda = 0.0;
    let mut n: i32 = 0;
    let mut did_bup = false;
    let mut ierr1 = 0;
    let mut label;

    if min(a, b) <= 1.0 {
        do_swap = x > 0.5;
        if do_swap {
            a0 = b;
            x0 = y;
            b0 = a;
            y0 = x;
        } else {
            a0 = a;
            x0 = x;
            b0 = b;
            y0 = y;
        }
        'pick: {
            if b0 < min(eps, eps * a0) {
                w = fpser(a0, b0, x0, eps, log_p);
                w1 = if log_p { r_log1_exp(w) } else { 0.5 - w + 0.5 };
                label = Label::End;
                break 'pick;
            }
            if a0 < min(eps, eps * b0) && b0 * x0 <= 1.0 {
                w1 = apser(a0, b0, x0, eps);
                label = Label::EndFromW1;
                break 'pick;
            }
            if max(a0, b0) > 1.0 {
                if b0 <= 1.0 {
                    label = Label::WBpser;
                    break 'pick;
                }
                if x0 >= 0.29 {
                    label = Label::W1Bpser;
                    break 'pick;
                }
                if x0 < 0.1 && (x0 * b0).powf(a0) <= 0.7 {
                    label = Label::WBpser;
                    break 'pick;
                }
                if b0 > 15.0 {
                    w1 = 0.0;
                    label = Label::L131;
                    break 'pick;
                }
            } else {
                if a0 >= min(0.2, b0) {
                    label = Label::WBpser;
                    break 'pick;
                }
                if x0.powf(a0) <= 0.9 {
                    label = Label::WBpser;
                    break 'pick;
                }
                if x0 >= 0.3 {
                    label = Label::W1Bpser;
                    break 'pick;
                }
            }
            n = 20;
            w1 = bup(b0, a0, y0, x0, n, eps, false);
            did_bup = true;
            b0 += n as f64;
            label = Label::L131;
        }
    } else {
        lambda =
            if (a + b).is_finite() { if a > b { (a + b) * y - b } else { a - (a + b) * x } } else { a * y - b * x };
        do_swap = lambda < 0.0;
        if do_swap {
            lambda = -lambda;
            a0 = b;
            x0 = y;
            b0 = a;
            y0 = x;
        } else {
            a0 = a;
            x0 = x;
            b0 = b;
            y0 = y;
        }
        'pick: {
            if b0 < 40.0 {
                if b0 * x0 <= 0.7 || (log_p && lambda > 650.0) {
                    label = Label::WBpser;
                } else {
                    label = Label::L140;
                }
                break 'pick;
            } else if a0 > b0 {
                if b0 <= 100.0 || lambda > b0 * 0.03 {
                    label = Label::Bfrac;
                    break 'pick;
                }
            } else if a0 <= 100.0 {
                label = Label::Bfrac;
                break 'pick;
            } else if lambda > a0 * 0.03 {
                label = Label::Bfrac;
                break 'pick;
            }
            w = basym(a0, b0, lambda, eps * 100.0, log_p);
            w1 = if log_p { r_log1_exp(w) } else { 0.5 - w + 0.5 };
            label = Label::End;
        }
    }

    loop {
        match label {
            Label::L131 => {
                bgrat(b0, a0, y0, x0, &mut w1, 15.0 * eps, &mut ierr1, false);
                if w1 == 0.0 || (0.0 < w1 && w1 < DBL_MIN) {
                    if did_bup {
                        w1 = bup(b0 - n as f64, a0, y0, x0, n, eps, true);
                    } else {
                        w1 = ML_NEGINF;
                    }
                    bgrat(b0, a0, y0, x0, &mut w1, 15.0 * eps, &mut ierr1, true);
                    if ierr1 != 0 {
                        ierr = 10 + ierr1;
                    }
                    label = Label::EndFromW1Log;
                    continue;
                }
                if ierr1 != 0 {
                    ierr = 10 + ierr1;
                }
                if w1 < 0.0 {
                    mathlib_warning(format!(
                        "bratio(a={}, b={}, x={}): bgrat() -> w1 = {}",
                        fmt_g(a),
                        fmt_g(b),
                        fmt_g(x),
                        fmt_g(w1)
                    ));
                }
                label = Label::EndFromW1;
            }
            Label::WBpser => {
                w = bpser(a0, b0, x0, eps, log_p);
                w1 = if log_p { r_log1_exp(w) } else { 0.5 - w + 0.5 };
                label = Label::End;
            }
            Label::W1Bpser => {
                w1 = bpser(b0, a0, y0, eps, log_p);
                w = if log_p { r_log1_exp(w1) } else { 0.5 - w1 + 0.5 };
                label = Label::End;
            }
            Label::Bfrac => {
                w = bfrac(a0, b0, x0, y0, lambda, eps * 15.0, log_p);
                w1 = if log_p { r_log1_exp(w) } else { 0.5 - w + 0.5 };
                label = Label::End;
            }
            Label::L140 => {
                n = b0 as i32;
                b0 -= n as f64;
                if b0 == 0.0 {
                    n -= 1;
                    b0 = 1.0;
                }
                w = bup(b0, a0, y0, x0, n, eps, false);
                if w < DBL_MIN && log_p {
                    b0 += n as f64;
                    label = Label::WBpser;
                    continue;
                }
                if x0 <= 0.7 {
                    w += bpser(a0, b0, x0, eps, false);
                    label = Label::EndFromW;
                    continue;
                }
                if a0 <= 15.0 {
                    n = 20;
                    w += bup(a0, b0, x0, y0, n, eps, false);
                    a0 += n as f64;
                }
                bgrat(a0, b0, x0, y0, &mut w, 15.0 * eps, &mut ierr1, false);
                if ierr1 != 0 {
                    ierr = 10 + ierr1;
                }
                label = Label::EndFromW;
            }
            Label::EndFromW => {
                if log_p {
                    w1 = log1p(-w);
                    w = w.ln();
                } else {
                    w1 = 0.5 - w + 0.5;
                }
                label = Label::End;
            }
            Label::EndFromW1 => {
                if log_p {
                    w = log1p(-w1);
                    w1 = w1.ln();
                } else {
                    w = 0.5 - w1 + 0.5;
                }
                label = Label::End;
            }
            Label::EndFromW1Log => {
                if log_p {
                    w = r_log1_exp(w1);
                } else {
                    w = -expm1(w1);
                    w1 = w1.exp();
                }
                label = Label::End;
            }
            Label::End => {
                if do_swap {
                    std::mem::swap(&mut w, &mut w1);
                }
                return (w, w1, ierr);
            }
        }
    }
}

/// R: nmath/toms708.c:fpser -- I_x(a,b) for b < min(eps, eps*a), x <= 0.5
fn fpser(a: f64, b: f64, x: f64, eps: f64, log_p: bool) -> f64 {
    let mut ans;
    if log_p {
        ans = a * x.ln();
    } else if a > eps * 0.001 {
        let t = a * x.ln();
        if t < exparg(1) {
            return 0.0;
        }
        ans = t.exp();
    } else {
        ans = 1.0;
    }
    if log_p {
        ans += b.ln() - a.ln();
    } else {
        ans *= b / a;
    }
    let tol = eps / a;
    let mut an = a + 1.0;
    let mut t = x;
    let mut s = t / an;
    loop {
        an += 1.0;
        t *= x;
        let c = t / an;
        s += c;
        if !(c.abs() > tol) {
            break;
        }
    }
    if log_p {
        ans += log1p(a * s);
    } else {
        ans *= a * s + 1.0;
    }
    ans
}

/// R: nmath/toms708.c:apser -- I_{1-x}(b,a) for very small a
fn apser(a: f64, b: f64, x: f64, eps: f64) -> f64 {
    const G: f64 = 0.577215664901533;
    let bx = b * x;
    let mut t = x - bx;
    let c = if b * eps <= 0.02 { x.ln() + psi(b) + G + t } else { bx.ln() + G + t };
    let tol = eps * 5.0 * c.abs();
    let mut j = 1.0;
    let mut s = 0.0;
    loop {
        j += 1.0;
        t *= x - bx / j;
        let aj = t / j;
        s += aj;
        if !(aj.abs() > tol) {
            break;
        }
    }
    -a * (c + s)
}

/// R: nmath/toms708.c:bpser -- power series for I_x(a,b)
fn bpser(a: f64, b: f64, x: f64, eps: f64, log_p: bool) -> f64 {
    if x == 0.0 {
        return r_d__0(log_p);
    }
    let mut ans;
    let mut c;
    let z;
    let a0 = min(a, b);
    if a0 >= 1.0 {
        let z = a * x.ln() - betaln(a, b);
        ans = if log_p { z - a.ln() } else { z.exp() / a };
    } else {
        let mut b0 = max(a, b);
        if b0 < 8.0 {
            if b0 <= 1.0 {
                if log_p {
                    ans = a * x.ln();
                } else {
                    ans = x.powf(a);
                    if ans == 0.0 {
                        return ans;
                    }
                }
                let apb = a + b;
                let zz = if apb > 1.0 {
                    let u = a + b - 1.0;
                    (gam1(u) + 1.0) / apb
                } else {
                    gam1(apb) + 1.0
                };
                c = (gam1(a) + 1.0) * (gam1(b) + 1.0) / zz;
                if log_p {
                    ans += (c * (b / apb)).ln();
                } else {
                    ans *= c * (b / apb);
                }
            } else {
                let mut u = gamln1(a0);
                let m = (b0 - 1.0) as i32;
                if m >= 1 {
                    c = 1.0;
                    for _ in 1..=m {
                        b0 += -1.0;
                        c *= b0 / (a0 + b0);
                    }
                    u += c.ln();
                }
                z = a * x.ln() - u;
                b0 += -1.0;
                let apb = a0 + b0;
                let t = if apb > 1.0 {
                    let u = a0 + b0 - 1.0;
                    (gam1(u) + 1.0) / apb
                } else {
                    gam1(apb) + 1.0
                };
                if log_p {
                    ans = z + (a0 / a).ln() + log1p(gam1(b0)) - t.ln();
                } else {
                    ans = z.exp() * (a0 / a) * (gam1(b0) + 1.0) / t;
                }
            }
        } else {
            let u = gamln1(a0) + algdiv(a0, b0);
            let z = a * x.ln() - u;
            if log_p {
                ans = z + (a0 / a).ln();
            } else {
                ans = a0 / a * z.exp();
            }
        }
    }
    if ans == r_d__0(log_p) || (!log_p && a <= eps * 0.1) {
        return ans;
    }

    let tol = eps / a;
    let mut n = 0.0;
    let mut sum = 0.0;
    let mut w;
    c = 1.0;
    loop {
        n += 1.0;
        c *= (0.5 - b / n + 0.5) * x;
        w = c / (a + n);
        sum += w;
        if !(n < 1e7 && w.abs() > tol) {
            break;
        }
    }
    if w.abs() > tol
        && ((log_p && !(a * sum > -1.0 && log1p(a * sum).abs() < eps * ans.abs()))
            || (!log_p && (a * sum + 1.0).abs() != 1.0))
    {
        mathlib_warning(format!(
            " bpser(a={}, b={}, x={},...) did not converge (n=1e7, |w|/tol={} > 1; A={})",
            fmt_g(a),
            fmt_g(b),
            fmt_g(x),
            fmt_g(w.abs() / tol),
            fmt_g(ans)
        ));
    }
    if log_p {
        if a * sum > -1.0 {
            ans += log1p(a * sum);
        } else {
            if ans > ML_NEGINF {
                mathlib_warning(format!(
                    "pbeta(*, log.p=TRUE) -> bpser(a={}, b={}, x={},...) underflow to -Inf",
                    fmt_g(a),
                    fmt_g(b),
                    fmt_g(x)
                ));
            }
            ans = ML_NEGINF;
        }
    } else if a * sum > -1.0 {
        ans *= a * sum + 1.0;
    } else {
        ans = 0.0;
    }
    ans
}

/// R: nmath/toms708.c:bup -- I_x(a,b) - I_x(a+n,b), n a positive integer
fn bup(a: f64, b: f64, x: f64, y: f64, n: i32, eps: f64, give_log: bool) -> f64 {
    let apb = a + b;
    let ap1 = a + 1.0;
    let mut mu;
    let mut d;
    if n > 1 && a >= 1.0 && apb >= ap1 * 1.1 {
        mu = exparg(1).abs() as i32;
        let k = exparg(0) as i32;
        if mu > k {
            mu = k;
        }
        d = (-(mu as f64)).exp();
    } else {
        mu = 0;
        d = 1.0;
    }
    let mut ret_val = if give_log { brcmp1(mu, a, b, x, y, true) - a.ln() } else { brcmp1(mu, a, b, x, y, false) / a };
    if n == 1 || (give_log && ret_val == ML_NEGINF) || (!give_log && ret_val == 0.0) {
        return ret_val;
    }
    let nm1 = n - 1;
    let mut w = d;
    let mut k = 0;
    if b > 1.0 {
        if y > 1e-4 {
            let r = (b - 1.0) * x / y - a;
            if r >= 1.0 {
                k = if r < nm1 as f64 { r as i32 } else { nm1 };
            }
        } else {
            k = nm1;
        }
        for i in 0..k {
            let l = i as f64;
            d *= (apb + l) / (ap1 + l) * x;
            w += d;
        }
    }
    for i in k..nm1 {
        let l = i as f64;
        d *= (apb + l) / (ap1 + l) * x;
        w += d;
        if d <= eps * w {
            break;
        }
    }
    if give_log {
        ret_val += w.ln();
    } else {
        ret_val *= w;
    }
    ret_val
}

/// R: nmath/toms708.c:bfrac -- continued fraction for I_x(a,b), a,b > 1
fn bfrac(a: f64, b: f64, x: f64, y: f64, lambda: f64, eps: f64, log_p: bool) -> f64 {
    if !lambda.is_finite() {
        return ML_NAN;
    }
    let brc = brcomp(a, b, x, y, log_p);
    if brc.is_nan() {
        ml_warning(ME_DOMAIN, "");
        return ML_NAN;
    }
    if !log_p && brc == 0.0 {
        return 0.0;
    }
    let c = lambda + 1.0;
    let c0 = b / a;
    let c1 = 1.0 / a + 1.0;
    let yp1 = y + 1.0;

    let mut n = 0.0;
    let mut p = 1.0;
    let mut s = a + 1.0;
    let mut an = 0.0;
    let mut bn = 1.0;
    let mut anp1 = 1.0;
    let mut bnp1 = c / c1;
    let mut r = c1 / c;
    let mut r0;

    loop {
        n += 1.0;
        let mut t = n / a;
        let w = n * (b - n) * x;
        let mut e = a / s;
        let alpha = p * (p + c0) * e * e * (w * x);
        e = (t + 1.0) / (c1 + t + t);
        let beta = n + w / s + e * (c + n * yp1);
        p = t + 1.0;
        s += 2.0;

        t = alpha * an + beta * anp1;
        an = anp1;
        anp1 = t;
        t = alpha * bn + beta * bnp1;
        bn = bnp1;
        bnp1 = t;

        r0 = r;
        r = anp1 / bnp1;
        if (r - r0).abs() <= eps * r {
            break;
        }
        an /= bnp1;
        bn /= bnp1;
        anp1 = r;
        bnp1 = 1.0;
        if n >= 10000.0 {
            break;
        }
    }
    if n >= 10000.0 && (r - r0).abs() > eps * r {
        mathlib_warning(format!(
            " bfrac(a={}, b={}, x={}, y={}, lambda={}) did *not* converge (in 10000 steps)\n",
            fmt_g(a),
            fmt_g(b),
            fmt_g(x),
            fmt_g(y),
            fmt_g(lambda)
        ));
    }
    if log_p { brc + r.ln() } else { brc * r }
}

/// R: nmath/toms708.c:brcomp -- x^a * y^b / Beta(a,b)
fn brcomp(a: f64, b: f64, x: f64, y: f64, log_p: bool) -> f64 {
    const CONST: f64 = 0.398942280401433;
    if x == 0.0 || y == 0.0 {
        return r_d__0(log_p);
    }
    let a0 = min(a, b);
    if a0 < 8.0 {
        let (lnx, lny);
        if x <= 0.375 {
            lnx = x.ln();
            lny = alnrel(-x);
        } else if y > 0.375 {
            lnx = x.ln();
            lny = y.ln();
        } else {
            lnx = alnrel(-y);
            lny = y.ln();
        }
        let mut z = a * lnx + b * lny;
        if a0 >= 1.0 {
            z -= betaln(a, b);
            return r_d_exp(z, log_p);
        }
        let mut b0 = max(a, b);
        if b0 >= 8.0 {
            let u = gamln1(a0) + algdiv(a0, b0);
            return if log_p { a0.ln() + (z - u) } else { a0 * (z - u).exp() };
        }
        if b0 <= 1.0 {
            let e_z = r_d_exp(z, log_p);
            if !log_p && e_z == 0.0 {
                return 0.0;
            }
            let apb = a + b;
            let zz = if apb > 1.0 {
                let u = a + b - 1.0;
                (gam1(u) + 1.0) / apb
            } else {
                gam1(apb) + 1.0
            };
            let c = (gam1(a) + 1.0) * (gam1(b) + 1.0) / zz;
            return if log_p { e_z + (a0 * c).ln() - log1p(a0 / b0) } else { e_z * (a0 * c) / (a0 / b0 + 1.0) };
        }
        let mut u = gamln1(a0);
        let n = (b0 - 1.0) as i32;
        if n >= 1 {
            let mut c = 1.0;
            for _ in 1..=n {
                b0 += -1.0;
                c *= b0 / (a0 + b0);
            }
            u = c.ln() + u;
        }
        z -= u;
        b0 += -1.0;
        let apb = a0 + b0;
        let t = if apb > 1.0 {
            let u = a0 + b0 - 1.0;
            (gam1(u) + 1.0) / apb
        } else {
            gam1(apb) + 1.0
        };
        if log_p { a0.ln() + z + log1p(gam1(b0)) - t.ln() } else { a0 * z.exp() * (gam1(b0) + 1.0) / t }
    } else {
        let (h, x0, y0, lambda);
        if a <= b {
            h = a / b;
            x0 = h / (h + 1.0);
            y0 = 1.0 / (h + 1.0);
            lambda = a - (a + b) * x;
        } else {
            h = b / a;
            x0 = 1.0 / (h + 1.0);
            y0 = h / (h + 1.0);
            lambda = (a + b) * y - b;
        }
        let mut e = -lambda / a;
        let u = if e.abs() > 0.6 { e - (x / x0).ln() } else { rlog1(e) };
        e = lambda / b;
        let v = if e.abs() <= 0.6 { rlog1(e) } else { e - (y / y0).ln() };
        let z = if log_p { -(a * u + b * v) } else { (-(a * u + b * v)).exp() };
        if log_p {
            -M_LN_SQRT_2PI + 0.5 * (b * x0).ln() + z - bcorr(a, b)
        } else {
            CONST * (b * x0).sqrt() * z * (-bcorr(a, b)).exp()
        }
    }
}

/// R: nmath/toms708.c:brcmp1 -- exp(mu) * x^a * y^b / beta(a,b)
fn brcmp1(mu: i32, a: f64, b: f64, x: f64, y: f64, give_log: bool) -> f64 {
    const CONST: f64 = 0.398942280401433;
    let a0 = min(a, b);
    if a0 < 8.0 {
        let (lnx, lny);
        if x <= 0.375 {
            lnx = x.ln();
            lny = alnrel(-x);
        } else if y > 0.375 {
            lnx = x.ln();
            lny = y.ln();
        } else {
            lnx = alnrel(-y);
            lny = y.ln();
        }
        let mut z = a * lnx + b * lny;
        if a0 >= 1.0 {
            z -= betaln(a, b);
            return esum(mu, z, give_log);
        }
        let mut b0 = max(a, b);
        if b0 >= 8.0 {
            let u = gamln1(a0) + algdiv(a0, b0);
            return if give_log { a0.ln() + esum(mu, z - u, true) } else { a0 * esum(mu, z - u, false) };
        } else if b0 <= 1.0 {
            let ans = esum(mu, z, give_log);
            if ans == (if give_log { ML_NEGINF } else { 0.0 }) {
                return ans;
            }
            let apb = a + b;
            let zz = if apb > 1.0 {
                let u = a + b - 1.0;
                (gam1(u) + 1.0) / apb
            } else {
                gam1(apb) + 1.0
            };
            let c = if give_log {
                log1p(gam1(a)) + log1p(gam1(b)) - zz.ln()
            } else {
                (gam1(a) + 1.0) * (gam1(b) + 1.0) / zz
            };
            return if give_log { ans + a0.ln() + c - log1p(a0 / b0) } else { ans * (a0 * c) / (a0 / b0 + 1.0) };
        }
        let mut u = gamln1(a0);
        let n = (b0 - 1.0) as i32;
        if n >= 1 {
            let mut c = 1.0;
            for _ in 1..=n {
                b0 += -1.0;
                c *= b0 / (a0 + b0);
            }
            u += c.ln();
        }
        z -= u;
        b0 += -1.0;
        let apb = a0 + b0;
        let t = if apb > 1.0 { (gam1(apb - 1.0) + 1.0) / apb } else { gam1(apb) + 1.0 };
        if give_log {
            a0.ln() + esum(mu, z, true) + log1p(gam1(b0)) - t.ln()
        } else {
            a0 * esum(mu, z, false) * (gam1(b0) + 1.0) / t
        }
    } else {
        let (h, x0, y0, lambda);
        if a > b {
            h = b / a;
            x0 = 1.0 / (h + 1.0);
            y0 = h / (h + 1.0);
            lambda = (a + b) * y - b;
        } else {
            h = a / b;
            x0 = h / (h + 1.0);
            y0 = 1.0 / (h + 1.0);
            lambda = a - (a + b) * x;
        }
        let lx0 = -log1p(b / a);
        let mut e = -lambda / a;
        let u = if e.abs() > 0.6 { e - (x / x0).ln() } else { rlog1(e) };
        e = lambda / b;
        let v = if e.abs() > 0.6 { e - (y / y0).ln() } else { rlog1(e) };
        let z = esum(mu, -(a * u + b * v), give_log);
        if give_log {
            CONST.ln() + (b.ln() + lx0) / 2.0 + z - bcorr(a, b)
        } else {
            CONST * (b * x0).sqrt() * z * (-bcorr(a, b)).exp()
        }
    }
}

/// R: nmath/toms708.c:bgrat -- asymptotic expansion for I_x(a,b), a >= 15,
/// b <= 1; computes `w := w + I_x(a,b)` (on log scale when `log_w`).
fn bgrat(a: f64, b: f64, x: f64, y: f64, w: &mut f64, eps: f64, ierr: &mut i32, log_w: bool) {
    const N_TERMS: usize = 30;
    let mut c = [0.0f64; N_TERMS];
    let mut d = [0.0f64; N_TERMS];
    let bm1 = b - 0.5 - 0.5;
    let nu = a + bm1 * 0.5;
    let lnx = if y > 0.375 { x.ln() } else { alnrel(-y) };
    let z = -nu * lnx;

    if b * z == 0.0 {
        mathlib_warning(format!(
            "bgrat(a={}, b={}, x={}, y={}): z={}, b*z == 0 underflow, hence inaccurate pbeta()",
            fmt_g(a),
            fmt_g(b),
            fmt_g(x),
            fmt_g(y),
            fmt_g(z)
        ));
        *ierr = 1;
        return;
    }

    let log_r = b.ln() + log1p(gam1(b)) + b * z.ln() + nu * lnx;
    let log_u = log_r - (algdiv(b, a) + b * nu.ln());
    let u = log_u.exp();

    if log_u == ML_NEGINF {
        *ierr = 2;
        return;
    }

    let u_0 = u == 0.0;
    let l = if log_w {
        if *w == ML_NEGINF { 0.0 } else { (*w - log_u).exp() }
    } else if *w == 0.0 {
        0.0
    } else {
        (w.ln() - log_u).exp()
    };
    let q_r = grat_r(b, z, log_r, eps);
    let v = 0.25 / (nu * nu);
    let t2 = lnx * 0.25 * lnx;
    let mut j = q_r;
    let mut sum = j;
    let mut t = 1.0;
    let mut cn = 1.0;
    let mut n2 = 0.0;
    for n in 1..=N_TERMS {
        let bp2n = b + n2;
        j = (bp2n * (bp2n + 1.0) * j + (z + bp2n + 1.0) * t) * v;
        n2 += 2.0;
        t *= t2;
        cn /= n2 * (n2 + 1.0);
        let nm1 = n - 1;
        c[nm1] = cn;
        let mut s = 0.0;
        if n > 1 {
            let mut coef = b - n as f64;
            for i in 1..=nm1 {
                s += coef * c[i - 1] * d[nm1 - i];
                coef += b;
            }
        }
        d[nm1] = bm1 * cn + s / n as f64;
        let dj = d[nm1] * j;
        sum += dj;
        if sum <= 0.0 {
            *ierr = 3;
            return;
        }
        if dj.abs() <= eps * (sum + l) {
            *ierr = 0;
            break;
        } else if n == N_TERMS {
            *ierr = 4;
            mathlib_warning(format!(
                "bgrat(a={}, b={}, x={}) *no* convergence: NOTIFY R-core!\n dj={}, rel.err={}\n",
                fmt_g(a),
                fmt_g(b),
                fmt_g(x),
                fmt_g(dj),
                fmt_g(dj.abs() / (sum + l))
            ));
        }
    }
    if log_w {
        *w = super::pgamma::logspace_add(*w, log_u + sum.ln());
    } else {
        *w += if u_0 { (log_u + sum.ln()).exp() } else { u * sum };
    }
}

/// R: nmath/toms708.c:grat_r -- Q(a,x) / r with r = e^-x x^a / Gamma(a)
fn grat_r(a: f64, x: f64, log_r: f64, eps: f64) -> f64 {
    if a * x == 0.0 {
        if x <= a {
            return (-log_r).exp();
        } else {
            return 0.0;
        }
    } else if a == 0.5 {
        if x < 0.25 {
            let p = erf__(x.sqrt());
            return (0.5 - p + 0.5) * (-log_r).exp();
        } else {
            let sx = x.sqrt();
            return erfc1(1, sx) / sx * M_SQRT_PI;
        }
    } else if x < 1.1 {
        let mut an = 3.0;
        let mut c = x;
        let mut sum = x / (a + 3.0);
        let tol = eps * 0.1 / (a + 1.0);
        let mut t;
        loop {
            an += 1.0;
            c *= -(x / an);
            t = c / (a + an);
            sum += t;
            if !(t.abs() > tol) {
                break;
            }
        }
        let j = a * x * ((sum / 6.0 - 0.5 / (a + 2.0)) * x + 1.0 / (a + 1.0));
        let z = a * x.ln();
        let h = gam1(a);
        let g = h + 1.0;
        if (x >= 0.25 && (a < x / 2.59)) || (z > -0.13394) {
            let l = rexpm1(z);
            let q = ((l + 0.5 + 0.5) * j - l) * g - h;
            if q <= 0.0 { 0.0 } else { q * (-log_r).exp() }
        } else {
            let p = z.exp() * g * (0.5 - j + 0.5);
            (0.5 - p + 0.5) * (-log_r).exp()
        }
    } else {
        let mut a2n_1 = 1.0;
        let mut a2n = 1.0;
        let mut b2n_1 = x;
        let mut b2n = x + (1.0 - a);
        let mut c = 1.0;
        let mut am0;
        let mut an0;
        loop {
            a2n_1 = x * a2n + c * a2n_1;
            b2n_1 = x * b2n + c * b2n_1;
            am0 = a2n_1 / b2n_1;
            c += 1.0;
            let c_a = c - a;
            a2n = a2n_1 + c_a * a2n;
            b2n = b2n_1 + c_a * b2n;
            an0 = a2n / b2n;
            if !((an0 - am0).abs() >= eps * an0) {
                break;
            }
        }
        an0
    }
}

/// R: nmath/toms708.c:basym -- asymptotic expansion for large a and b
fn basym(a: f64, b: f64, lambda: f64, eps: f64, log_p: bool) -> f64 {
    const NUM_IT: usize = 20;
    const E0: f64 = 1.12837916709551;
    const E1: f64 = 0.353553390593274;
    const LN_E0: f64 = 0.120782237635245;

    let mut a0 = [0.0f64; NUM_IT + 1];
    let mut b0 = [0.0f64; NUM_IT + 1];
    let mut c = [0.0f64; NUM_IT + 1];
    let mut d = [0.0f64; NUM_IT + 1];

    let f = a * rlog1(-lambda / a) + b * rlog1(lambda / b);
    let t;
    if log_p {
        t = -f;
    } else {
        t = (-f).exp();
        if t == 0.0 {
            return 0.0;
        }
    }
    let z0 = f.sqrt();
    let z = z0 / E1 * 0.5;
    let z2 = f + f;
    let (h, r0, r1, w0);
    if a < b {
        h = a / b;
        r0 = 1.0 / (h + 1.0);
        r1 = (b - a) / b;
        w0 = 1.0 / (a * (h + 1.0)).sqrt();
    } else {
        h = b / a;
        r0 = 1.0 / (h + 1.0);
        r1 = (b - a) / a;
        w0 = 1.0 / (b * (h + 1.0)).sqrt();
    }
    a0[0] = r1 * 0.66666666666666663;
    c[0] = a0[0] * -0.5;
    d[0] = -c[0];
    let mut j0 = 0.5 / E0 * erfc1(1, z0);
    let mut j1 = E1;
    let mut sum = j0 + d[0] * w0 * j1;

    let mut s = 1.0;
    let h2 = h * h;
    let mut hn = 1.0;
    let mut w = w0;
    let mut znm1 = z;
    let mut zn = z2;
    let mut n = 2;
    while n <= NUM_IT {
        hn *= h2;
        a0[n - 1] = r0 * 2.0 * (h * hn + 1.0) / (n as f64 + 2.0);
        let np1 = n + 1;
        s += hn;
        a0[np1 - 1] = r1 * 2.0 * s / (n as f64 + 3.0);

        for i in n..=np1 {
            let r = (i as f64 + 1.0) * -0.5;
            b0[0] = r * a0[0];
            for m in 2..=i {
                let mut bsum = 0.0;
                for j in 1..=(m - 1) {
                    let mmj = m - j;
                    bsum += (j as f64 * r - mmj as f64) * a0[j - 1] * b0[mmj - 1];
                }
                b0[m - 1] = r * a0[m - 1] + bsum / m as f64;
            }
            c[i - 1] = b0[i - 1] / (i as f64 + 1.0);
            let mut dsum = 0.0;
            for j in 1..i {
                dsum += d[i - j - 1] * c[j - 1];
            }
            d[i - 1] = -(dsum + c[i - 1]);
        }

        j0 = E1 * znm1 + (n as f64 - 1.0) * j0;
        j1 = E1 * zn + n as f64 * j1;
        znm1 *= z2;
        zn *= z2;
        w *= w0;
        let t0 = d[n - 1] * w * j0;
        w *= w0;
        let t1 = d[np1 - 1] * w * j1;
        sum += t0 + t1;
        if t0.abs() + t1.abs() <= eps * sum {
            break;
        }
        n += 2;
    }
    if log_p {
        LN_E0 + t - bcorr(a, b) + sum.ln()
    } else {
        let u = (-bcorr(a, b)).exp();
        E0 * t * u * sum
    }
}

/// R: nmath/toms708.c:exparg
fn exparg(l: i32) -> f64 {
    const LNB: f64 = 0.69314718055995;
    let m = if l == 0 { DBL_MAX_EXP } else { DBL_MIN_EXP - 1 };
    m as f64 * LNB * 0.99999
}

/// R: nmath/toms708.c:esum -- exp(mu + x)
fn esum(mu: i32, x: f64, give_log: bool) -> f64 {
    if give_log {
        return x + mu as f64;
    }
    let w;
    if x > 0.0 {
        if mu > 0 {
            return (mu as f64).exp() * x.exp();
        }
        w = mu as f64 + x;
        if w < 0.0 {
            return (mu as f64).exp() * x.exp();
        }
    } else {
        if mu < 0 {
            return (mu as f64).exp() * x.exp();
        }
        w = mu as f64 + x;
        if w > 0.0 {
            return (mu as f64).exp() * x.exp();
        }
    }
    w.exp()
}

/// R: nmath/toms708.c:rexpm1 -- exp(x) - 1
pub fn rexpm1(x: f64) -> f64 {
    const P1: f64 = 9.14041914819518e-10;
    const P2: f64 = 0.0238082361044469;
    const Q1: f64 = -0.499999999085958;
    const Q2: f64 = 0.107141568980644;
    const Q3: f64 = -0.0119041179760821;
    const Q4: f64 = 5.95130811860248e-4;
    if x.abs() <= 0.15 {
        x * (((P2 * x + P1) * x + 1.0) / ((((Q4 * x + Q3) * x + Q2) * x + Q1) * x + 1.0))
    } else {
        let w = x.exp();
        if x > 0.0 { w * (0.5 - 1.0 / w + 0.5) } else { w - 0.5 - 0.5 }
    }
}

/// R: nmath/toms708.c:alnrel -- ln(1 + a)
fn alnrel(a: f64) -> f64 {
    if a.abs() > 0.375 {
        return (1.0 + a).ln();
    }
    const P1: f64 = -1.29418923021993;
    const P2: f64 = 0.405303492862024;
    const P3: f64 = -0.0178874546012214;
    const Q1: f64 = -1.62752256355323;
    const Q2: f64 = 0.747811014037616;
    const Q3: f64 = -0.0845104217945565;
    let t = a / (a + 2.0);
    let t2 = t * t;
    let w = (((P3 * t2 + P2) * t2 + P1) * t2 + 1.0) / (((Q3 * t2 + Q2) * t2 + Q1) * t2 + 1.0);
    t * 2.0 * w
}

/// R: nmath/toms708.c:rlog1 -- x - ln(1 + x)
fn rlog1(x: f64) -> f64 {
    const A: f64 = 0.0566749439387324;
    const B: f64 = 0.0456512608815524;
    const P0: f64 = 0.333333333333333;
    const P1: f64 = -0.224696413112536;
    const P2: f64 = 0.00620886815375787;
    const Q1: f64 = -1.27408923933623;
    const Q2: f64 = 0.354508718369557;

    if !(-0.39..=0.57).contains(&x) {
        let w = x + 0.5 + 0.5;
        return x - w.ln();
    }
    let (h, w1);
    if x < -0.18 {
        let hh = x + 0.3;
        h = hh / 0.7;
        w1 = A - h * 0.3;
    } else if x > 0.18 {
        h = x * 0.75 - 0.25;
        w1 = B + h / 3.0;
    } else {
        h = x;
        w1 = 0.0;
    }
    let r = h / (h + 2.0);
    let t = r * r;
    let w = ((P2 * t + P1) * t + P0) / ((Q2 * t + Q1) * t + 1.0);
    t * 2.0 * (1.0 / (1.0 - r) - r * w) + w1
}

const ERF_C: f64 = 0.564189583547756;
const ERF_A: [f64; 5] =
    [7.7105849500132e-5, -0.00133733772997339, 0.0323076579225834, 0.0479137145607681, 0.128379167095513];
const ERF_B: [f64; 3] = [0.00301048631703895, 0.0538971687740286, 0.375795757275549];
const ERF_P: [f64; 8] = [
    -1.36864857382717e-7,
    0.564195517478974,
    7.21175825088309,
    43.1622272220567,
    152.98928504694,
    339.320816734344,
    451.918953711873,
    300.459261020162,
];
const ERF_Q: [f64; 8] = [
    1.0,
    12.7827273196294,
    77.0001529352295,
    277.585444743988,
    638.980264465631,
    931.35409485061,
    790.950925327898,
    300.459260956983,
];
const ERF_R: [f64; 5] = [2.10144126479064, 26.2370141675169, 21.3688200555087, 4.6580782871847, 0.282094791773523];
const ERF_S: [f64; 4] = [94.153775055546, 187.11481179959, 99.0191814623914, 18.0124575948747];

/// R: nmath/toms708.c:erf__ -- the real error function
fn erf__(x: f64) -> f64 {
    let (a, b, p, q, r, s) = (&ERF_A, &ERF_B, &ERF_P, &ERF_Q, &ERF_R, &ERF_S);
    let ax = x.abs();
    if ax <= 0.5 {
        let t = x * x;
        let top = (((a[0] * t + a[1]) * t + a[2]) * t + a[3]) * t + a[4] + 1.0;
        let bot = ((b[0] * t + b[1]) * t + b[2]) * t + 1.0;
        return x * (top / bot);
    }
    if ax <= 4.0 {
        let top = ((((((p[0] * ax + p[1]) * ax + p[2]) * ax + p[3]) * ax + p[4]) * ax + p[5]) * ax + p[6]) * ax + p[7];
        let bot = ((((((q[0] * ax + q[1]) * ax + q[2]) * ax + q[3]) * ax + q[4]) * ax + q[5]) * ax + q[6]) * ax + q[7];
        let rr = 0.5 - (-x * x).exp() * top / bot + 0.5;
        return if x < 0.0 { -rr } else { rr };
    }
    if ax >= 5.8 {
        return if x > 0.0 { 1.0 } else { -1.0 };
    }
    let x2 = x * x;
    let mut t = 1.0 / x2;
    let top = (((r[0] * t + r[1]) * t + r[2]) * t + r[3]) * t + r[4];
    let bot = (((s[0] * t + s[1]) * t + s[2]) * t + s[3]) * t + 1.0;
    t = (ERF_C - top / (x2 * bot)) / ax;
    let rr = 0.5 - (-x2).exp() * t + 0.5;
    if x < 0.0 { -rr } else { rr }
}

/// R: nmath/toms708.c:erfc1 -- erfc(x), or exp(x^2) * erfc(x) if ind != 0
fn erfc1(ind: i32, x: f64) -> f64 {
    let (a, b, p, q, r, s) = (&ERF_A, &ERF_B, &ERF_P, &ERF_Q, &ERF_R, &ERF_S);
    let mut ret_val;
    let ax = x.abs();
    if ax <= 0.5 {
        let t = x * x;
        let top = (((a[0] * t + a[1]) * t + a[2]) * t + a[3]) * t + a[4] + 1.0;
        let bot = ((b[0] * t + b[1]) * t + b[2]) * t + 1.0;
        ret_val = 0.5 - x * (top / bot) + 0.5;
        if ind != 0 {
            ret_val *= t.exp();
        }
        return ret_val;
    }
    if ax <= 4.0 {
        let top = ((((((p[0] * ax + p[1]) * ax + p[2]) * ax + p[3]) * ax + p[4]) * ax + p[5]) * ax + p[6]) * ax + p[7];
        let bot = ((((((q[0] * ax + q[1]) * ax + q[2]) * ax + q[3]) * ax + q[4]) * ax + q[5]) * ax + q[6]) * ax + q[7];
        ret_val = top / bot;
    } else {
        if x <= -5.6 {
            ret_val = 2.0;
            if ind != 0 {
                ret_val = (x * x).exp() * 2.0;
            }
            return ret_val;
        }
        if ind == 0 && (x > 100.0 || x * x > -exparg(1)) {
            return 0.0;
        }
        let t = 1.0 / (x * x);
        let top = (((r[0] * t + r[1]) * t + r[2]) * t + r[3]) * t + r[4];
        let bot = (((s[0] * t + s[1]) * t + s[2]) * t + s[3]) * t + 1.0;
        ret_val = (ERF_C - t * top / bot) / ax;
    }
    if ind != 0 {
        if x < 0.0 {
            ret_val = (x * x).exp() * 2.0 - ret_val;
        }
    } else {
        let w = x * x;
        let t = w;
        let e = w - t;
        ret_val = (0.5 - e + 0.5) * (-t).exp() * ret_val;
        if x < 0.0 {
            ret_val = 2.0 - ret_val;
        }
    }
    ret_val
}

/// R: nmath/toms708.c:gam1 -- 1/gamma(a+1) - 1 for -0.5 <= a <= 1.5
fn gam1(a: f64) -> f64 {
    let mut t = a;
    let d = a - 0.5;
    if d > 0.0 {
        t = d - 0.5;
    }
    if t < 0.0 {
        const R: [f64; 9] = [
            -0.422784335098468,
            -0.771330383816272,
            -0.244757765222226,
            0.118378989872749,
            9.30357293360349e-4,
            -0.0118290993445146,
            0.00223047661158249,
            2.66505979058923e-4,
            -1.32674909766242e-4,
        ];
        const S1: f64 = 0.273076135303957;
        const S2: f64 = 0.0559398236957378;
        let top =
            (((((((R[8] * t + R[7]) * t + R[6]) * t + R[5]) * t + R[4]) * t + R[3]) * t + R[2]) * t + R[1]) * t + R[0];
        let bot = (S2 * t + S1) * t + 1.0;
        let w = top / bot;
        if d > 0.0 { t * w / a } else { a * (w + 0.5 + 0.5) }
    } else if t == 0.0 {
        0.0
    } else {
        const P: [f64; 7] = [
            0.577215664901533,
            -0.409078193005776,
            -0.230975380857675,
            0.0597275330452234,
            0.0076696818164949,
            -0.00514889771323592,
            5.89597428611429e-4,
        ];
        const Q: [f64; 5] = [1.0, 0.427569613095214, 0.158451672430138, 0.0261132021441447, 0.00423244297896961];
        let top = (((((P[6] * t + P[5]) * t + P[4]) * t + P[3]) * t + P[2]) * t + P[1]) * t + P[0];
        let bot = (((Q[4] * t + Q[3]) * t + Q[2]) * t + Q[1]) * t + 1.0;
        let w = top / bot;
        if d > 0.0 { t / a * (w - 0.5 - 0.5) } else { a * w }
    }
}

/// R: nmath/toms708.c:gamln1 -- ln(gamma(1 + a)) for -0.2 <= a <= 1.25
fn gamln1(a: f64) -> f64 {
    if a < 0.6 {
        const P0: f64 = 0.577215664901533;
        const P1: f64 = 0.844203922187225;
        const P2: f64 = -0.168860593646662;
        const P3: f64 = -0.780427615533591;
        const P4: f64 = -0.402055799310489;
        const P5: f64 = -0.0673562214325671;
        const P6: f64 = -0.00271935708322958;
        const Q1: f64 = 2.88743195473681;
        const Q2: f64 = 3.12755088914843;
        const Q3: f64 = 1.56875193295039;
        const Q4: f64 = 0.361951990101499;
        const Q5: f64 = 0.0325038868253937;
        const Q6: f64 = 6.67465618796164e-4;
        let w = ((((((P6 * a + P5) * a + P4) * a + P3) * a + P2) * a + P1) * a + P0)
            / ((((((Q6 * a + Q5) * a + Q4) * a + Q3) * a + Q2) * a + Q1) * a + 1.0);
        -a * w
    } else {
        const R0: f64 = 0.422784335098467;
        const R1: f64 = 0.848044614534529;
        const R2: f64 = 0.565221050691933;
        const R3: f64 = 0.156513060486551;
        const R4: f64 = 0.017050248402265;
        const R5: f64 = 4.97958207639485e-4;
        const S1: f64 = 1.24313399877507;
        const S2: f64 = 0.548042109832463;
        const S3: f64 = 0.10155218743983;
        const S4: f64 = 0.00713309612391;
        const S5: f64 = 1.16165475989616e-4;
        let x = a - 0.5 - 0.5;
        let w = (((((R5 * x + R4) * x + R3) * x + R2) * x + R1) * x + R0)
            / (((((S5 * x + S4) * x + S3) * x + S2) * x + S1) * x + 1.0);
        x * w
    }
}

/// R: nmath/toms708.c:psi -- the digamma function (Cody, Strecok, Thacher)
fn psi(x: f64) -> f64 {
    const PIOV4: f64 = 0.785398163397448;
    const DX0: f64 = 1.461632144968362341262659542325721325;
    const P1: [f64; 7] = [
        0.0089538502298197,
        4.77762828042627,
        142.441585084029,
        1186.45200713425,
        3633.51846806499,
        4138.10161269013,
        1305.60269827897,
    ];
    const Q1: [f64; 6] =
        [44.8452573429826, 520.752771467162, 2210.0079924783, 3641.27349079381, 1908.310765963, 6.91091682714533e-6];
    const P2: [f64; 4] = [-2.12940445131011, -7.01677227766759, -4.48616543918019, -0.648157123766197];
    const Q2: [f64; 4] = [32.2703493791143, 89.2920700481861, 54.6117738103215, 7.77788548522962];

    let mut x = x;
    let mut xmax1 = INT_MAX as f64;
    let d2 = 0.5 / D1MACH3;
    if xmax1 > d2 {
        xmax1 = d2;
    }
    let xsmall = 1e-9;
    let mut aug = 0.0;
    if x < 0.5 {
        if x.abs() <= xsmall {
            if x == 0.0 {
                return 0.0;
            }
            aug = -1.0 / x;
        } else {
            let mut w = -x;
            let mut sgn = PIOV4;
            if w <= 0.0 {
                w = -w;
                sgn = -sgn;
            }
            if w >= xmax1 {
                return 0.0;
            }
            let mut nq = w as i32;
            w -= nq as f64;
            nq = (w * 4.0) as i32;
            w = (w - nq as f64 * 0.25) * 4.0;
            let mut n = nq / 2;
            if n + n != nq {
                w = 1.0 - w;
            }
            let z = PIOV4 * w;
            let mut m = n / 2;
            if m + m != n {
                sgn = -sgn;
            }
            n = (nq + 1) / 2;
            m = n / 2;
            m += m;
            if m == n {
                if z == 0.0 {
                    return 0.0;
                }
                aug = sgn * (z.cos() / z.sin() * 4.0);
            } else {
                aug = sgn * (z.sin() / z.cos() * 4.0);
            }
        }
        x = 1.0 - x;
    }
    if x <= 3.0 {
        let mut den = x;
        let mut upper = P1[0] * x;
        for i in 1..=5 {
            den = (den + Q1[i - 1]) * x;
            upper = (upper + P1[i]) * x;
        }
        den = (upper + P1[6]) / (den + Q1[5]);
        let xmx0 = x - DX0;
        return den * xmx0 + aug;
    }
    if x < xmax1 {
        let w = 1.0 / (x * x);
        let mut den = w;
        let mut upper = P2[0] * w;
        for i in 1..=3 {
            den = (den + Q2[i - 1]) * w;
            upper = (upper + P2[i]) * w;
        }
        aug = upper / (den + Q2[3]) - 0.5 / x + aug;
    }
    aug + x.ln()
}

/// R: nmath/toms708.c:betaln -- ln(beta(a0, b0))
fn betaln(a0: f64, b0: f64) -> f64 {
    let mut a = min(a0, b0);
    let b = max(a0, b0);
    if a < 8.0 {
        if a < 1.0 {
            if b < 8.0 {
                return gamln(a) + (gamln(b) - gamln(a + b));
            } else {
                return gamln(a) + algdiv(a, b);
            }
        }
        let w;
        if a < 2.0 {
            if b <= 2.0 {
                return gamln(a) + gamln(b) - gsumln(a, b);
            }
            if b < 8.0 {
                w = 0.0;
                // goto L40
                return betaln_l40(a, b, w);
            }
            return gamln(a) + algdiv(a, b);
        }
        if b <= 1e3 {
            let n = (a - 1.0) as i32;
            let mut ww = 1.0;
            for _ in 1..=n {
                a += -1.0;
                let h = a / b;
                ww *= h / (h + 1.0);
            }
            let w = ww.ln();
            if b >= 8.0 {
                return w + gamln(a) + algdiv(a, b);
            }
            betaln_l40(a, b, w)
        } else {
            let n = (a - 1.0) as i32;
            let mut w = 1.0;
            for _ in 1..=n {
                a += -1.0;
                w *= a / (a / b + 1.0);
            }
            w.ln() - n as f64 * b.ln() + (gamln(a) + algdiv(a, b))
        }
    } else {
        const E: f64 = 0.918938533204673;
        let w = bcorr(a, b);
        let h = a / b;
        let u = -(a - 0.5) * (h / (h + 1.0)).ln();
        let v = b * alnrel(h);
        if u > v { b.ln() * -0.5 + E + w - v - u } else { b.ln() * -0.5 + E + w - u - v }
    }
}

/// betaln() label L40: 1 < a <= b < 8, reduction of b
fn betaln_l40(a: f64, mut b: f64, w: f64) -> f64 {
    let n = (b - 1.0) as i32;
    let mut z = 1.0;
    for _ in 1..=n {
        b += -1.0;
        z *= b / (a + b);
    }
    w + z.ln() + (gamln(a) + (gamln(b) - gsumln(a, b)))
}

/// R: nmath/toms708.c:gsumln -- ln(gamma(a + b)) for 1 <= a,b <= 2
fn gsumln(a: f64, b: f64) -> f64 {
    let x = a + b - 2.0;
    if x <= 0.25 {
        return gamln1(x + 1.0);
    }
    if x <= 1.25 {
        return gamln1(x) + alnrel(x);
    }
    gamln1(x - 1.0) + (x * (x + 1.0)).ln()
}

const C0: f64 = 0.0833333333333333;
const C1: f64 = -0.00277777777760991;
const C2: f64 = 7.9365066682539e-4;
const C3: f64 = -5.9520293135187e-4;
const C4: f64 = 8.37308034031215e-4;
const C5: f64 = -0.00165322962780713;

/// R: nmath/toms708.c:bcorr -- del(a0) + del(b0) - del(a0 + b0)
fn bcorr(a0: f64, b0: f64) -> f64 {
    let a = min(a0, b0);
    let b = max(a0, b0);
    let h = a / b;
    let c = h / (h + 1.0);
    let x = 1.0 / (h + 1.0);
    let x2 = x * x;
    let s3 = x + x2 + 1.0;
    let s5 = x + x2 * s3 + 1.0;
    let s7 = x + x2 * s5 + 1.0;
    let s9 = x + x2 * s7 + 1.0;
    let s11 = x + x2 * s9 + 1.0;
    let mut t = 1.0 / b;
    t *= t;
    let mut w = ((((C5 * s11 * t + C4 * s9) * t + C3 * s7) * t + C2 * s5) * t + C1 * s3) * t + C0;
    w *= c / b;
    t = 1.0 / a;
    t *= t;
    (((((C5 * t + C4) * t + C3) * t + C2) * t + C1) * t + C0) / a + w
}

/// R: nmath/toms708.c:algdiv -- ln(gamma(b)/gamma(a+b)) when b >= 8
fn algdiv(a: f64, b: f64) -> f64 {
    let (h, c, x, d);
    if a > b {
        h = b / a;
        c = 1.0 / (h + 1.0);
        x = h / (h + 1.0);
        d = a + (b - 0.5);
    } else {
        h = a / b;
        c = h / (h + 1.0);
        x = 1.0 / (h + 1.0);
        d = b + (a - 0.5);
    }
    let x2 = x * x;
    let s3 = x + x2 + 1.0;
    let s5 = x + x2 * s3 + 1.0;
    let s7 = x + x2 * s5 + 1.0;
    let s9 = x + x2 * s7 + 1.0;
    let s11 = x + x2 * s9 + 1.0;
    let t = 1.0 / (b * b);
    let mut w = ((((C5 * s11 * t + C4 * s9) * t + C3 * s7) * t + C2 * s5) * t + C1 * s3) * t + C0;
    w *= c / b;
    let u = d * alnrel(a / b);
    let v = a * (b.ln() - 1.0);
    if u > v { w - v - u } else { w - u - v }
}

/// R: nmath/toms708.c:gamln -- ln(gamma(a)) for positive a
fn gamln(a: f64) -> f64 {
    const D: f64 = 0.418938533204673;
    if a <= 0.8 {
        gamln1(a) - a.ln()
    } else if a <= 2.25 {
        gamln1(a - 0.5 - 0.5)
    } else if a < 10.0 {
        let n = (a - 1.25) as i32;
        let mut t = a;
        let mut w = 1.0;
        for _ in 1..=n {
            t += -1.0;
            w *= t;
        }
        gamln1(t - 1.0) + w.ln()
    } else {
        let t = 1.0 / (a * a);
        let w = (((((C5 * t + C4) * t + C3) * t + C2) * t + C1) * t + C0) / a;
        D + w + (a - 0.5) * (a.ln() - 1.0)
    }
}
