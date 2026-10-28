//! The noncentral chi-squared distribution: dnchisq.c, pnchisq.c,
//! qnchisq.c.

use super::gamma::lgammafn;
use super::ldmath::expl;
use super::ldouble::LDouble;
use super::pgamma::{dchisq, dpois_raw, logspace_add, pchisq, qchisq};
use super::*;

/// R: nmath/dnchisq.c:dnchisq
pub fn dnchisq(x: f64, df: f64, ncp: f64, give_log: bool) -> f64 {
    const EPS: f64 = 5e-15;

    if x.is_nan() || df.is_nan() || ncp.is_nan() {
        return x + df + ncp;
    }
    if !df.is_finite() || !ncp.is_finite() || ncp < 0.0 || df < 0.0 {
        return ML_NAN;
    }
    if x < 0.0 {
        return r_d__0(give_log);
    }
    if x == 0.0 && df < 2.0 {
        return ML_POSINF;
    }
    if ncp == 0.0 {
        return if df > 0.0 { dchisq(x, df, give_log) } else { r_d__0(give_log) };
    }
    if x == ML_POSINF {
        return r_d__0(give_log);
    }

    let ncp2 = 0.5 * ncp;

    // find max element of sum
    let mut imax = ((-(2.0 + df) + ((2.0 - df) * (2.0 - df) + 4.0 * ncp * x).sqrt()) / 4.0).ceil();
    if imax < 0.0 {
        imax = 0.0;
    }
    let mut dfmid = 0.0;
    let mid = if imax.is_finite() {
        dfmid = df + 2.0 * imax;
        dpois_raw(imax, ncp2, false) * dchisq(x, dfmid, false)
    } else {
        // imax = Inf
        0.0
    };

    if mid == 0.0 {
        // underflow to 0 -- maybe numerically correct; maybe can be more
        // accurate, particularly when give_log = TRUE
        // Use central-chisq approximation formula when appropriate;
        if give_log || ncp > 1000.0 {
            let nl = df + ncp;
            let ic = nl / (nl + ncp); // = "1/(1+b)" Abramowitz & St.*
            return dchisq(x * ic, nl * ic, give_log);
        } else {
            return r_d__0(give_log);
        }
    }

    let mut sum = LDouble::from_f64(mid);

    // errorbound := term * q / (1-q)  now subsumed in while() / if() below:

    // upper tail
    let mut term = LDouble::from_f64(mid);
    let mut df = dfmid;
    let mut i = imax;
    let x2 = x * ncp2;
    let mut q;
    loop {
        i += 1.0;
        q = x2 / i / df;
        df += 2.0;
        term *= q;
        sum += term;
        if !(q >= 1.0 || term * q > LDouble::from_f64((1.0 - q) * EPS) || term > sum * 1e-10) {
            break;
        }
    }
    // lower tail
    term = LDouble::from_f64(mid);
    df = dfmid;
    i = imax;
    while i != 0.0 {
        df -= 2.0;
        q = i * df / x2;
        i -= 1.0;
        term *= q;
        sum += term;
        if q < 1.0 && term * q <= LDouble::from_f64((1.0 - q) * EPS) {
            break;
        }
    }
    r_d_val(sum.to_f64(), give_log)
}

/// R: nmath/pnchisq.c:pnchisq
pub fn pnchisq(x: f64, df: f64, ncp: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || df.is_nan() || ncp.is_nan() {
        return x + df + ncp;
    }
    if !df.is_finite() || !ncp.is_finite() {
        return ML_NAN;
    }
    if df < 0.0 || ncp < 0.0 {
        return ML_NAN;
    }

    let mut ans = pnchisq_raw(x, df, ncp, 1e-12, 8.0 * DBL_EPSILON, 1000000, lower_tail, log_p);

    if x <= 0.0 || x == ML_POSINF {
        return ans; // because it's perfect
    }

    if ncp >= 80.0 {
        if lower_tail {
            ans = fmin2(ans, r_d__1(log_p)); // e.g., pchisq(555, 1.01, ncp = 80)
        } else {
            // since we computed the other tail cancellation is likely
            if ans < if log_p { -10.0 * M_LN10 } else { 1e-10 } {
                ml_warning(ME_PRECISION, "pnchisq");
            }
            if !log_p && ans < 0.0 {
                ans = 0.0; // Precaution PR#7099
            }
        }
    }
    // MM: the following "hack" from c51179 (<--> PR#14216, by Jerry Lewis)
    // -- is "kind of ok" ... but potentially suboptimal: we do
    // log1p(- p(*, <other tail>, log=FALSE)), but that p(*, log=FALSE) may
    // already be an exp(.) or even expm1(..)
    if !log_p || ans < -1e-8 {
        ans
    } else {
        // log_p (==> ans <= 0) && -1e-8 <= ans <= 0
        // prob. = exp(ans) is near one: we can do better using the other tail
        ans = pnchisq_raw(x, df, ncp, 1e-12, 8.0 * DBL_EPSILON, 1000000, !lower_tail, false);
        log1p(-ans)
    }
}

/// `_dbl_min_exp = M_LN2 * DBL_MIN_EXP` = -708.3964 for IEEE double precision
fn dbl_min_exp() -> f64 {
    M_LN2 * DBL_MIN_EXP as f64
}

/// R: nmath/pnchisq.c:pnchisq_raw
#[allow(clippy::too_many_arguments)]
pub fn pnchisq_raw(
    x: f64,
    f: f64,
    theta: f64,
    errmax: f64,
    reltol: f64,
    itrmax: i32,
    lower_tail: bool,
    log_p: bool,
) -> f64 {
    if x <= 0.0 {
        if x == 0.0 && f == 0.0 {
            // chi^2_0(.) has point mass at zero
            let l = -0.5 * theta; // = -lambda
            return if lower_tail {
                r_d_exp(l, log_p)
            } else if log_p {
                r_log1_exp(l)
            } else {
                -expm1(l)
            };
        }
        // x < 0  or {x==0, f > 0}
        return r_dt_0(lower_tail, log_p);
    }
    if !x.is_finite() {
        return r_dt_1(lower_tail, log_p);
    }

    if theta < 80.0 {
        // use 110 for Inf, as ppois(110, 80/2, lower.tail=FALSE) is 2e-20
        // Have  pgamma(x,s) < x^s / Gamma(s+1) (< and ~= for small x)
        // ==> pchisq(x, f) = pgamma(x, f/2, 2) = pgamma(x/2, f/2)
        //                  <  (x/2)^(f/2) / Gamma(f/2+1) < eps
        // <==>  f/2 * log(x/2) - log(Gamma(f/2+1)) < log(eps) ( ~= -708.3964 )
        // <==>        log(x/2) < 2/f*(log(Gamma(f/2+1)) + log(eps))
        // <==> log(x) < log(2) + 2/f*(log(Gamma(f/2+1)) + log(eps))
        if lower_tail && f > 0.0 && x.ln() < M_LN2 + 2.0 / f * (lgammafn(f / 2.0 + 1.0) + dbl_min_exp()) {
            // all pchisq(x, f+2*i, lower_tail, FALSE), i=0,...,110 would
            // underflow to 0. ==> work in log scale
            let lambda = 0.5 * theta; // < 40
            let mut sum = ML_NEGINF;
            let mut sum2 = ML_NEGINF;
            let mut pr = -lambda;
            let log_lam = lambda.ln();
            let mut i = 0;
            while i < 110 {
                sum2 = logspace_add(sum2, pr);
                sum = logspace_add(sum, pr + pchisq(x, f + 2.0 * i as f64, lower_tail, true));
                if sum2 >= -1e-15 {
                    // <=> EXP(sum2) >= 1-1e-15
                    break;
                }
                i += 1;
                pr += log_lam - (i as f64).ln();
            }
            let ans = sum - sum2;
            return if log_p { ans } else { ans.exp() };
        } else {
            let lambda = LDouble::from_f64(0.5 * theta); // < 40
            let mut sum = LDouble::from_f64(0.0);
            let mut sum2 = LDouble::from_f64(0.0);
            let mut pr = expl(LDouble::from_f64(0.0) - lambda);
            // we need to renormalize here: the result could be very close to 1
            let mut i = 0;
            while i < 110 {
                // pr == exp(-lambda) lambda^i / i!  ==  dpois(i, lambda)
                sum2 += pr;
                // pchisq(*, i, *) is strictly decreasing to 0 for
                // lower_tail=TRUE and strictly increasing to 1 for
                // lower_tail=FALSE
                sum += pr * pchisq(x, f + 2.0 * i as f64, lower_tail, false);
                if sum2 >= LDouble::from_f64(1.0 - 1e-15) {
                    break;
                }
                i += 1;
                pr *= lambda / i as f64;
            }
            let ans = sum / sum2;
            return if log_p { super::ldmath::logl(ans).to_f64() } else { ans.to_f64() };
        }
    } // if(theta < 80)

    // else: theta == ncp >= 80 --------------------------------------------
    // Series expansion ---- FIXME: log_p=TRUE, lower_tail=FALSE only applied
    // at end ==> underflow
    let lam = 0.5 * theta; // = lambda = ncp/2
    let mut lam_sml = -lam < dbl_min_exp();
    let mut u;
    let mut lu = -1.0;
    let mut l_lam = -1.0;
    if lam_sml {
        // originally error: "non centrality parameter too large for current algorithm"
        u = LDouble::from_f64(0.0);
        lu = -lam; // == ln(u)
        l_lam = lam.ln();
    } else {
        u = LDouble::from_f64((-lam).exp());
    }

    // evaluate the first term
    let mut v = u;
    let x2 = 0.5 * x;
    let f2 = 0.5 * f;
    let mut f_x_2n = f - x;

    let mut lt;
    let t0 = x2 - f2;
    if f2 * DBL_EPSILON > 0.125 && t0.abs() < DBL_EPSILON.sqrt() * f2 {
        // very large f and x ~= f: probably needs another algorithm anyway
        // evade cancellation error
        // t = exp((1 - t)*(2 - t/(f2 + 1))) / sqrt(2*M_PI*(f2 + 1));
        lt = (1.0 - t0) * (2.0 - t0 / (f2 + 1.0)) - M_LN_SQRT_2PI - 0.5 * (f2 + 1.0).ln();
    } else {
        // Usual case 2: careful not to overflow .. :
        lt = f2 * x2.ln() - x2 - lgammafn(f2 + 1.0);
    }

    let mut t_sml = lt < dbl_min_exp();
    let mut t;
    let mut ans;
    let mut term;
    let mut l_x = 0.0;
    if t_sml {
        if x > f + theta + 5.0 * (2.0 * (f + 2.0 * theta)).sqrt() {
            // x > E[X] + 5* sigma(X)
            return r_dt_1(lower_tail, log_p); // FIXME: could be more accurate than 0.
        }
        // else
        l_x = x.ln();
        ans = LDouble::from_f64(0.0);
        term = LDouble::from_f64(0.0);
        t = LDouble::from_f64(0.0);
    } else {
        t = expl(LDouble::from_f64(lt));
        ans = LDouble::from_f64((v * t).to_f64());
        term = ans;
    }

    let mut n = 1;
    let mut f_2n = f + 2.0;
    f_x_2n += 2.0;
    while n <= itrmax {
        // f_2n    === f + 2*n
        // f_x_2n  === f - x + 2*n   > 0  <==> (f+2n)  >   x
        if f_x_2n > 0.0 {
            // find the error bound and check for convergence
            let bound = (t * x / f_x_2n).to_f64();
            // convergence only if BOTH absolute and relative error < 'bnd'
            let is_b = bound <= errmax;
            let is_r = term <= ans * reltol;
            if is_b && is_r {
                break; // out completely
            }
        }

        // evaluate the next term of the expansion and then the partial sum
        if lam_sml {
            lu += l_lam - (n as f64).ln(); // u = u* lam / n
            if lu >= dbl_min_exp() {
                // no underflow anymore ==> change regime
                v = expl(LDouble::from_f64(lu)); // the first non-0 'u'
                u = v;
                lam_sml = false;
            }
        } else {
            u *= lam / n as f64;
            v += u;
        }
        if t_sml {
            lt += l_x - f_2n.ln(); // t <- t * (x / f2n)
            if lt >= dbl_min_exp() {
                // no underflow anymore ==> change regime
                t = expl(LDouble::from_f64(lt)); // the first non-0 't'
                t_sml = false;
            }
        } else {
            t *= x / f_2n;
        }
        if !lam_sml && !t_sml {
            term = LDouble::from_f64((v * t).to_f64());
            ans += term;
        }
        n += 1;
        f_2n += 2.0;
        f_x_2n += 2.0;
    }

    if n > itrmax {
        mathlib_warning(format!(
            "pnchisq(x={}, f={}, theta={}, ..): not converged in {} iter.",
            fmt_g(x),
            fmt_g(f),
            fmt_g(theta),
            itrmax
        ));
    }

    let dans = ans.to_f64();
    r_dt_val(dans, lower_tail, log_p)
}

/// R: nmath/qnchisq.c:qnchisq
pub fn qnchisq(p: f64, df: f64, ncp: f64, lower_tail: bool, log_p: bool) -> f64 {
    const ACCU: f64 = 1e-13;
    const RACC: f64 = 4.0 * DBL_EPSILON;
    // these two are for the "search" loops, can have less accuracy:
    const EPS: f64 = 1e-11; // must be > accu
    const REPS: f64 = 1e-10; // relative tolerance ...

    if p.is_nan() || df.is_nan() || ncp.is_nan() {
        return p + df + ncp;
    }
    if !df.is_finite() {
        return ML_NAN;
    }
    // Was
    // df = floor(df + 0.5);
    // if (df < 1 || ncp < 0) ML_WARN_return_NAN;
    if df < 0.0 || ncp < 0.0 {
        return ML_NAN;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, ML_POSINF, lower_tail, log_p) {
        return v;
    }

    let pp = r_d_qiv(p, log_p); // exp(p) iff log_p
    if pp > 1.0 - DBL_EPSILON {
        return if lower_tail { ML_POSINF } else { 0.0 }; // early under/over flow iff log_p (FIXME)
    }

    // Invert pnchisq(.):
    // 1. finding an upper and lower bound
    // This is Pearson's (1959) approximation, which is usually good to 4 figs or so.
    let ux0;
    let mut ux;
    {
        let b = (ncp * ncp) / (df + 3.0 * ncp);
        let c = (df + 3.0 * ncp) / (df + 2.0 * ncp);
        let ff = (df + 2.0 * ncp) / (c * c);
        ux = b + c * qchisq(p, ff, lower_tail, log_p);
        if ux <= 0.0 {
            ux = 1.0;
        }
        ux0 = ux;
    }

    let mut p = p;
    let mut lower_tail = lower_tail;
    if !lower_tail && ncp >= 80.0 {
        // in this case, pnchisq() works via lower_tail = TRUE
        if pp < 1e-10 {
            ml_warning(ME_PRECISION, "qnchisq");
        }
        p = if log_p { -expm1(p) } else { 0.5 - p + 0.5 };
        lower_tail = true;
    } else {
        p = pp;
    }

    let mut pp = fmin2(1.0 - DBL_EPSILON, p * (1.0 + EPS));
    let mut lx;
    if lower_tail {
        while ux < DBL_MAX && pnchisq_raw(ux, df, ncp, EPS, REPS, 10000, true, false) < pp {
            ux *= 2.0;
        }
        pp = p * (1.0 - EPS);
        lx = fmin2(ux0, DBL_MAX);
        while lx > DBL_MIN && pnchisq_raw(lx, df, ncp, EPS, REPS, 10000, true, false) > pp {
            lx *= 0.5;
        }
    } else {
        while ux < DBL_MAX && pnchisq_raw(ux, df, ncp, EPS, REPS, 10000, false, false) > pp {
            ux *= 2.0;
        }
        pp = p * (1.0 - EPS);
        lx = fmin2(ux0, DBL_MAX);
        while lx > DBL_MIN && pnchisq_raw(lx, df, ncp, EPS, REPS, 10000, false, false) < pp {
            lx *= 0.5;
        }
    }

    // 2. interval (lx,ux)  halving:
    loop {
        let nx = 0.5 * (lx + ux);
        if lower_tail {
            if pnchisq_raw(nx, df, ncp, ACCU, RACC, 100000, true, false) > p {
                ux = nx;
            } else {
                lx = nx;
            }
        } else if pnchisq_raw(nx, df, ncp, ACCU, RACC, 100000, false, false) < p {
            ux = nx;
        } else {
            lx = nx;
        }
        if !((ux - lx) / nx > ACCU) {
            break;
        }
    }
    0.5 * (ux + lx)
}
