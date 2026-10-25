//! The beta distribution: dbeta.c, pbeta.c, qbeta.c.

#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::binom::dbinom_raw;
use super::gamma::lbeta;
use super::toms708::bratio;
use super::*;

/// R: nmath/dbeta.c:dbeta
pub fn dbeta(x: f64, a: f64, b: f64, give_log: bool) -> f64 {
    if x.is_nan() || a.is_nan() || b.is_nan() {
        return x + a + b;
    }
    if a < 0.0 || b < 0.0 {
        return ML_NAN;
    }
    if !(0.0..=1.0).contains(&x) {
        return r_d__0(give_log);
    }
    if a == 0.0 || b == 0.0 || !a.is_finite() || !b.is_finite() {
        if a == 0.0 && b == 0.0 {
            return if x == 0.0 || x == 1.0 { ML_POSINF } else { r_d__0(give_log) };
        }
        if a == 0.0 || a / b == 0.0 {
            return if x == 0.0 { ML_POSINF } else { r_d__0(give_log) };
        }
        if b == 0.0 || b / a == 0.0 {
            return if x == 1.0 { ML_POSINF } else { r_d__0(give_log) };
        }
        return if x == 0.5 { ML_POSINF } else { r_d__0(give_log) };
    }
    if x == 0.0 {
        if a > 1.0 {
            return r_d__0(give_log);
        }
        if a < 1.0 {
            return ML_POSINF;
        }
        return r_d_val(b, give_log);
    }
    if x == 1.0 {
        if b > 1.0 {
            return r_d__0(give_log);
        }
        if b < 1.0 {
            return ML_POSINF;
        }
        return r_d_val(a, give_log);
    }
    let lval = if a <= 2.0 || b <= 2.0 {
        (a - 1.0) * x.ln() + (b - 1.0) * log1p(-x) - lbeta(a, b)
    } else {
        (a + b - 1.0).ln() + dbinom_raw(a - 1.0, a + b - 2.0, x, 1.0 - x, true)
    };
    r_d_exp(lval, give_log)
}

/// R: nmath/pbeta.c:pbeta_raw
pub fn pbeta_raw(x: f64, a: f64, b: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x >= 1.0 {
        return r_dt_1(lower_tail, log_p);
    }
    if a == 0.0 || b == 0.0 || !a.is_finite() || !b.is_finite() {
        if a == 0.0 && b == 0.0 {
            return if log_p { -M_LN2 } else { 0.5 };
        }
        if a == 0.0 || a / b == 0.0 {
            return r_dt_1(lower_tail, log_p);
        }
        if b == 0.0 || b / a == 0.0 {
            return r_dt_0(lower_tail, log_p);
        }
        return if x < 0.5 { r_dt_0(lower_tail, log_p) } else { r_dt_1(lower_tail, log_p) };
    }
    if x <= 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    let x1 = 0.5 - x + 0.5;
    let (w, wc, ierr) = bratio(a, b, x, x1, log_p);
    if ierr != 0 && ierr != 11 && ierr != 14 {
        mathlib_warning(format!(
            "pbeta_raw({}, a={}, b={}, lower={}, log={}) -> bratio() gave error code {}",
            fmt_g(x),
            fmt_g(a),
            fmt_g(b),
            lower_tail as i32,
            log_p as i32,
            ierr
        ));
    }
    if lower_tail { w } else { wc }
}

/// R: nmath/pbeta.c:pbeta
pub fn pbeta(x: f64, a: f64, b: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || a.is_nan() || b.is_nan() {
        return x + a + b;
    }
    if a < 0.0 || b < 0.0 {
        return ML_NAN;
    }
    pbeta_raw(x, a, b, lower_tail, log_p)
}

const USE_LOG_X_CUTOFF: f64 = -5.0;
const N_NEWTON_FREE: i32 = 4;

/// R: nmath/qbeta.c:qbeta
pub fn qbeta(alpha: f64, p: f64, q: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || q.is_nan() || alpha.is_nan() {
        return p + q + alpha;
    }
    if p < 0.0 || q < 0.0 {
        return ML_NAN;
    }
    qbeta_raw(alpha, p, q, lower_tail, log_p, USE_LOG_X_CUTOFF, N_NEWTON_FREE)[0]
}

const DBL_VERY_MIN: f64 = DBL_MIN / 4.0;
const DBL_LOG_V_MIN: f64 = M_LN2 * (DBL_MIN_EXP - 2) as f64;
/// 0x1.fffffffffffffp-1 = 1 - 2^-53
const DBL_1_EPS: f64 = 1.0 - DBL_EPSILON / 2.0;
const FPU: f64 = 3e-308;
const ACU_MIN: f64 = 1e-300;
const P_LO: f64 = FPU;
const P_HI: f64 = 1.0 - 2.22e-16;
const CONST1: f64 = 2.30753;
const CONST2: f64 = 0.27061;
const CONST3: f64 = 0.99229;
const CONST4: f64 = 0.04481;

/// C's `%.<prec>g`.
fn fmt_g_prec(x: f64, prec: usize) -> String {
    if !x.is_finite() || x == 0.0 {
        return fmt_g(x);
    }
    let p = prec.max(1);
    let e = format!("{:.*e}", p - 1, x);
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let strip = |s: &str| -> String {
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() }
    };
    if exp < -4 || exp >= p as i32 {
        format!("{}e{}{:02}", strip(mant), if exp < 0 { '-' } else { '+' }, exp.abs())
    } else {
        let dec = (p as i32 - 1 - exp).max(0) as usize;
        strip(&format!("{:.*}", dec, x))
    }
}

/// R: nmath/qbeta.c:qbeta_raw. Returns `[qbeta(), 1 - qbeta()]` (on log
/// scale when `log_q_cut == +Inf`).
#[allow(unused_assignments)]
pub fn qbeta_raw(
    alpha: f64,
    p: f64,
    q: f64,
    lower_tail: bool,
    log_p: bool,
    log_q_cut: f64,
    n_n: i32,
) -> [f64; 2] {
    let give_log_q = log_q_cut == ML_POSINF;
    let mut use_log_x = give_log_q;
    let mut warned = false;
    let mut add_n_step = true;
    let mut y = -1.0;

    let q_0 = || if give_log_q { [ML_NEGINF, 0.0] } else { [0.0, 1.0] };
    let q_1 = || if give_log_q { [0.0, ML_NEGINF] } else { [1.0, 0.0] };
    let q_half = || if give_log_q { [-M_LN2, -M_LN2] } else { [0.5, 0.5] };

    if alpha == r_dt_0(lower_tail, log_p) {
        return q_0();
    }
    if alpha == r_dt_1(lower_tail, log_p) {
        return q_1();
    }
    if (log_p && alpha > 0.0) || (!log_p && !(0.0..=1.0).contains(&alpha)) {
        ml_warning(ME_DOMAIN, "");
        return [ML_NAN, ML_NAN];
    }
    if p == 0.0 || q == 0.0 || !p.is_finite() || !q.is_finite() {
        if p == 0.0 && q == 0.0 {
            if alpha < r_d_half(log_p) {
                return q_0();
            }
            if alpha > r_d_half(log_p) {
                return q_1();
            }
            return q_half();
        } else if p == 0.0 || p / q == 0.0 {
            return q_0();
        } else if q == 0.0 || q / p == 0.0 {
            return q_1();
        }
        return q_half();
    }

    let p_ = r_dt_qiv(alpha, lower_tail, log_p);
    let logbeta = lbeta(p, q);
    let mut swap_tail = p_ > 0.5;
    let mut n_maybe_swaps = 0;

    let (mut a, mut la, mut pp, mut qq);
    // (uninitialized in C where unused)
    let (mut r, mut t, mut w, mut u, mut xinbta, mut tx, mut u_n) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0);

    'maybe_swap: loop {
        if swap_tail {
            a = r_dt_civ(alpha, lower_tail, log_p);
            la = r_dt_clog(alpha, lower_tail, log_p);
            pp = q;
            qq = p;
        } else {
            a = p_;
            la = r_dt_log(alpha, lower_tail, log_p);
            pp = p;
            qq = q;
        }
        n_maybe_swaps += 1;

        let acu = fmax2(ACU_MIN, 10f64.powf(-13.0 - 2.5 / (pp * pp) - 0.5 / (a * a)));
        let u0 = (la + pp.ln() + logbeta) / pp;
        let mut rp = pp * (1.0 - qq) / (pp + 1.0);
        let log_eps_c = M_LN2 * (1.0 - DBL_MANT_DIG as f64);

        t = 0.2;
        let u0_maybe = M_LN2 * (DBL_MIN_EXP as f64) < u0 && u0 < -0.01;
        u_n = 1.0;

        let mut skip_to_return = false;
        'init: {
            if u0_maybe && u0 < (t * log_eps_c - (pp * (1.0 - qq) * (2.0 - qq) / (2.0 * (pp + 2.0))).abs().ln()) / 2.0 {
                rp *= u0.exp();
                if rp > -1.0 {
                    u = u0 - log1p(rp) / pp;
                } else {
                    u = u0;
                }
                xinbta = u.exp();
                tx = xinbta;
                use_log_x = true;
                break 'init;
            }

            r = (-2.0 * la).sqrt();
            y = r - (CONST1 + CONST2 * r) / (1.0 + (CONST3 + CONST4 * r) * r);
            if pp > 1.0 && qq > 1.0 {
                r = (y * y - 3.0) / 6.0;
                let s = 1.0 / (pp + pp - 1.0);
                t = 1.0 / (qq + qq - 1.0);
                let h = 2.0 / (s + t);
                w = y * (h + r).sqrt() / h - (t - s) * (r + 5.0 / 6.0 - 2.0 / (3.0 * h));
                if w > 300.0 {
                    t = w + w + qq.ln() - pp.ln();
                    u = if t <= 18.0 { -log1p(t.exp()) } else { -t - (-t).exp() };
                    xinbta = u.exp();
                } else {
                    xinbta = pp / (pp + qq * (w + w).exp());
                    u = -log1p(qq / pp * (w + w).exp());
                }
            } else {
                r = qq + qq;
                t = 1.0 / (3.0 * qq.sqrt());
                t = r * r_pow_di(1.0 + t * (-t + y), 3);
                let s = 4.0 * pp + r - 2.0;
                if t == 0.0 || (t < 0.0 && s >= t) {
                    let l1ma =
                        if swap_tail { r_dt_log(alpha, lower_tail, log_p) } else { r_dt_clog(alpha, lower_tail, log_p) };
                    let xx = (l1ma + qq.ln() + logbeta) / qq;
                    if xx <= 0.0 {
                        xinbta = -expm1(xx);
                        u = r_log1_exp(xx);
                    } else {
                        let r_ = rp * u0.exp();
                        if r_ > -1.0 {
                            u = u0 - log1p(r_) / pp;
                        } else {
                            u = u0;
                        }
                        xinbta = u.exp();
                    }
                } else {
                    t = s / t;
                    if t <= 1.0 {
                        u = u0;
                        xinbta = u.exp();
                    } else {
                        xinbta = 1.0 - 2.0 / (t + 1.0);
                        u = log1p(-2.0 / (t + 1.0));
                    }
                }
            }

            if (swap_tail && u >= -log_q_cut.exp())
                || (!swap_tail && u >= -(4.0 * log_q_cut).exp() && pp / qq < 1000.0)
            {
                swap_tail = !swap_tail;
                if swap_tail {
                    a = r_dt_civ(alpha, lower_tail, log_p);
                    la = r_dt_clog(alpha, lower_tail, log_p);
                    pp = q;
                    qq = p;
                } else {
                    a = p_;
                    la = r_dt_log(alpha, lower_tail, log_p);
                    pp = p;
                    qq = q;
                }
                u = r_log1_exp(u);
                xinbta = u.exp();
            }

            if !use_log_x {
                use_log_x = u < log_q_cut;
            }
            let bad_u = !u.is_finite();
            let bad_init = bad_u || xinbta > P_HI;

            tx = xinbta;

            if bad_u || u < log_q_cut {
                w = pbeta_raw(DBL_VERY_MIN, pp, qq, true, log_p);
                if w > (if log_p { la } else { a }) {
                    if log_p || (w - a).abs() < (0.0 - a).abs() {
                        tx = DBL_VERY_MIN;
                        u_n = DBL_LOG_V_MIN;
                    } else {
                        tx = 0.0;
                        u_n = ML_NEGINF;
                    }
                    use_log_x = log_p;
                    add_n_step = false;
                    skip_to_return = true;
                    break 'init;
                } else if u < DBL_LOG_V_MIN {
                    u = DBL_LOG_V_MIN;
                    xinbta = DBL_VERY_MIN;
                }
            }

            if bad_init && !(use_log_x && tx > 0.0) {
                if u == ML_NEGINF {
                    u = M_LN2 * DBL_MIN_EXP as f64;
                    xinbta = DBL_MIN;
                } else {
                    xinbta = if xinbta > 1.1 {
                        0.5
                    } else if xinbta < P_LO {
                        u.exp()
                    } else {
                        P_HI
                    };
                    if bad_u {
                        u = xinbta.ln();
                    }
                }
            }
        }

        if !skip_to_return {
            // L_Newton:
            r = 1.0 - pp;
            t = 1.0 - qq;
            let mut wprev: f64 = 0.0;
            let mut prev: f64 = 1.0;
            let mut adj: f64 = 1.0;

            let converged = 'newton: {
                if use_log_x {
                    for i_pb in 0..1000 {
                        y = pbeta_raw(xinbta, pp, qq, true, true);
                        w = if y == ML_NEGINF {
                            0.0
                        } else {
                            (y - la) * (y - u + logbeta + r * u + t * r_log1_exp(u)).exp()
                        };
                        if !w.is_finite() {
                            if n_maybe_swaps <= 1 {
                                continue 'maybe_swap;
                            }
                            ml_warning(ME_DOMAIN, "");
                            return [ML_NAN, ML_NAN];
                        }
                        if i_pb >= n_n && w * wprev <= 0.0 {
                            prev = fmax2(adj.abs(), FPU);
                        }
                        let mut g = 1.0;
                        for _ in 0..1000 {
                            adj = g * w;
                            if adj.abs() < prev {
                                u_n = u - adj;
                                if u_n <= 0.0 {
                                    if prev <= acu || w.abs() <= acu {
                                        break 'newton true;
                                    }
                                    break;
                                }
                            }
                            g /= 3.0;
                        }
                        let d = fmin2(adj.abs(), (u_n - u).abs());
                        if d <= 4e-16 * (u_n + u).abs() {
                            break 'newton true;
                        }
                        u = u_n;
                        xinbta = u.exp();
                        wprev = w;
                    }
                } else {
                    for i_pb in 0..1000 {
                        y = pbeta_raw(xinbta, pp, qq, true, log_p);
                        w = if log_p {
                            (y - la) * (y + logbeta + r * xinbta.ln() + t * log1p(-xinbta)).exp()
                        } else {
                            (y - a) * (logbeta + r * xinbta.ln() + t * log1p(-xinbta)).exp()
                        };
                        if !w.is_finite() {
                            if n_maybe_swaps <= 2 {
                                if !log_p && n_maybe_swaps == 2 {
                                    use_log_x = true;
                                }
                                if !log_p || n_maybe_swaps <= 1 {
                                    continue 'maybe_swap;
                                }
                            }
                            ml_warning(ME_DOMAIN, "");
                            return [ML_NAN, ML_NAN];
                        }
                        if i_pb >= n_n && w * wprev <= 0.0 {
                            prev = fmax2(adj.abs(), FPU);
                        }
                        let mut g = 1.0;
                        for _ in 0..1000 {
                            adj = g * w;
                            if i_pb < n_n || adj.abs() < prev {
                                tx = xinbta - adj;
                                if (0.0..=1.0).contains(&tx) {
                                    if prev <= acu || w.abs() <= acu {
                                        break 'newton true;
                                    }
                                    if tx != 0.0 && tx != 1.0 {
                                        break;
                                    }
                                }
                            }
                            g /= 3.0;
                        }
                        if (tx - xinbta).abs() <= 4e-16 * (tx + xinbta) {
                            break 'newton true;
                        }
                        xinbta = tx;
                        if tx == 0.0 {
                            break;
                        }
                        wprev = w;
                    }
                }
                false
            };
            if !converged {
                warned = true;
                ml_warning(ME_PRECISION, "qbeta");
            }

            // L_converged:
            let log_ = log_p || use_log_x;
            if (log_ && y == ML_NEGINF) || (!log_ && y == 0.0) {
                w = pbeta_raw(DBL_VERY_MIN, pp, qq, true, log_);
                if log_ || (w - a).abs() <= (y - a).abs() {
                    tx = DBL_VERY_MIN;
                    u_n = DBL_LOG_V_MIN;
                }
                add_n_step = false;
            } else if !warned && (if log_ { (y - la).abs() > 3.0 } else { (y - a).abs() > 1e-4 }) {
                if !(log_ && y == ML_NEGINF && pbeta_raw(DBL_1_EPS, pp, qq, true, true) > la + 2.0) {
                    mathlib_warning(format!(
                        "qbeta(a, *) =: x0 with |pbeta(x0,*{}) - alpha| = {} is not accurate",
                        if log_ { ", log_" } else { "" },
                        fmt_g_prec((y - if log_ { la } else { a }).abs(), 5)
                    ));
                }
            }
        }

        // L_return:
        if give_log_q {
            if !use_log_x {
                mathlib_warning(format!(
                    "qbeta() L_return, u_n={};  give_log_q=TRUE but use_log_x=FALSE -- please report!",
                    fmt_g(u_n)
                ));
            }
            let rr = r_log1_exp(u_n);
            return if swap_tail { [rr, u_n] } else { [u_n, rr] };
        }
        if use_log_x {
            if add_n_step {
                if u_n != 1.0 {
                    xinbta = u_n.exp();
                }
                y = pbeta_raw(xinbta, pp, qq, true, log_p);
                w = if log_p {
                    (y - la) * (y + logbeta + r * xinbta.ln() + t * log1p(-xinbta)).exp()
                } else {
                    (y - a) * (logbeta + r * xinbta.ln() + t * log1p(-xinbta)).exp()
                };
                tx = if w.is_finite() { xinbta - w } else { xinbta };
            } else {
                return if swap_tail { [-expm1(u_n), u_n.exp()] } else { [u_n.exp(), -expm1(u_n)] };
            }
        }
        return if swap_tail { [1.0 - tx, tx] } else { [tx, 1.0 - tx] };
    }
}

#[cfg(test)]
mod tests {
    use super::super::binom::pbinom;
    use super::*;

    /// Collects mismatches so that one run reports every failing value.
    macro_rules! chk {
        ($bad:ident, $e:expr, $v:expr) => {{
            let got: f64 = $e;
            let want: f64 = $v;
            if !(got == want || (got.is_nan() && want.is_nan())) {
                $bad.push(format!("{} = {:?} (want {:?})", stringify!($e), got, want));
            }
        }};
    }

    // Reference values: GNU R 4.5.0, sprintf("%.17g", .Internal(pbeta(...)))
    #[test]
    fn pbeta_values() {
        let mut bad: Vec<String> = Vec::new();
        chk!(bad, pbeta(0.3, 2.0, 5.0, true, false), 0.57982500000000015);
        chk!(bad, pbeta(0.3, 2.0, 5.0, false, false), 0.42017499999999985);
        chk!(bad, pbeta(0.3, 2.0, 5.0, true, true), -0.54502894510748867);
        chk!(bad, pbeta(0.3, 2.0, 5.0, false, true), -0.86708398781950713);
        chk!(bad, pbeta(0.5, 0.5, 0.5, true, false), 0.49999999999999956);
        chk!(bad, pbeta(0.01, 0.1, 10.0, true, false), 0.82448967090669867);
        chk!(bad, pbeta(0.999, 50.0, 0.5, true, false), 0.75236901996537675);
        chk!(bad, pbeta(0.999, 50.0, 0.5, false, true), -1.3958156246957114);
        chk!(bad, pbeta(0.2, 1e-20, 1e-20, true, false), 0.5);
        chk!(bad, pbeta(0.2, 1e-20, 2e-20, true, true), -0.40546510810816444);
        chk!(bad, pbeta(0.2, 1e-10, 1e-10, true, false), 0.49999999993068528);
        chk!(bad, pbeta(0.3, 1e-17, 0.5, true, false), 1.0);
        chk!(bad, pbeta(0.3, 0.5, 1e-17, true, false), 1.2302440093121528e-17);
        chk!(bad, pbeta(0.3, 1e-17, 3.0, false, false), 2.5897280432593478e-18);
        chk!(bad, pbeta(0.5, 1000.0, 1000.0, true, false), 0.5);
        chk!(bad, pbeta(0.45, 10000.0, 10000.0, true, false), 6.3126547777495716e-46);
        chk!(bad, pbeta(0.45, 10000.0, 10000.0, true, true), -104.07635796417776);
        chk!(bad, pbeta(0.4, 200.0, 300.0, true, false), 0.50242861631993174);
        chk!(bad, pbeta(0.4, 150.0, 220.0, false, false), 0.581313697602711);
        chk!(bad, pbeta(0.41, 150.0, 220.0, true, true), -0.55527199101854752);
        chk!(bad, pbeta(0.9, 30.0, 0.8, true, false), 0.027834227073053903);
        chk!(bad, pbeta(0.9, 30.0, 0.8, false, true), -0.028228940784651189);
        chk!(bad, pbeta(0.7, 25.0, 0.9, true, false), 0.000101565501505811);
        chk!(bad, pbeta(0.2, 3.0, 50.5, true, false), 0.99919429338435539);
        chk!(bad, pbeta(0.75, 3.5, 30.0, true, false), 0.99999999999999933);
        chk!(bad, pbeta(0.75, 3.5, 30.0, true, true), -7.4148057774846246e-16);
        chk!(bad, pbeta(0.6, 25.5, 12.0, true, false), 0.14713223113741172);
        chk!(bad, pbeta(0.35, 0.5, 20.0, true, false), 0.99996310914102526);
        chk!(bad, pbeta(0.05, 0.5, 20.0, true, false), 0.8454092185665617);
        chk!(bad, pbeta(0.5, 0.8, 0.3, true, false), 0.23939033139820723);
        chk!(bad, pbeta(0.95, 0.15, 0.3, true, false), 0.85503298486998158);
        chk!(bad, pbeta(0.25, 0.15, 0.3, true, false), 0.587562183674216);
        chk!(bad, pbeta(0.1, 60.0, 70.0, true, false), 2.780119219781367e-26);
        chk!(bad, pbeta(0.1, 60.0, 70.0, true, true), -58.844718606248726);
        chk!(bad, pbeta(0.01, 600.0, 700.0, true, true), -1877.3012171146995);
        chk!(bad, pbeta(9.99988867182683e-321, 1e-05, 0.5, true, false), 0.99995077031517055);
        chk!(bad, pbeta(1e-200, 2.0, 3.0, true, true), -919.24227772839026);
        chk!(bad, pbeta(0.5, 0.0, 0.0, true, false), 0.5);
        chk!(bad, pbeta(0.5, 0.0, 3.0, true, false), 1.0);
        chk!(bad, pbeta(0.5, 3.0, 0.0, true, false), 0.0);
        chk!(bad, pbeta(0.4, f64::INFINITY, f64::INFINITY, true, false), 0.0);
        chk!(bad, pbeta(0.4, 2.0, f64::INFINITY, false, false), 0.0);
        chk!(bad, pbeta(0.0, 2.0, 3.0, true, false), 0.0);
        chk!(bad, pbeta(1.0, 2.0, 3.0, false, true), f64::NEG_INFINITY);
        chk!(bad, pbeta(0.3, -1.0, 3.0, true, false), f64::NAN);
        chk!(bad, pbeta(0.5, 1e+15, 1e+15, true, false), 0.5);
        chk!(bad, pbeta(0.8, 0.005, 2e-04, true, false), 0.03872675704455724);
        chk!(bad, pbeta(0.3, 2e-16, 3e-16, true, false), 0.59999999999999987);
        chk!(bad, pbinom(3.0, 10.0, 0.3, true, false), 0.64961071840000018);
        chk!(bad, pbinom(50.0, 100.0, 0.5, false, false), 0.46020538130641092);
        chk!(bad, pbinom(5.0, 1000.0, 0.02, true, true), -9.6564858145930668);
        chk!(bad, pbinom(990.0, 1000.0, 0.98, false, true), -5.36429683331424);
        assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
    }

    #[test]
    fn dbeta_values() {
        let mut bad: Vec<String> = Vec::new();
        chk!(bad, dbeta(0.3, 2.0, 5.0, false), 2.1609);
        chk!(bad, dbeta(0.3, 2.0, 5.0, true), 0.77052480158128978);
        chk!(bad, dbeta(0.0, 1.0, 3.0, false), 3.0);
        chk!(bad, dbeta(1.0, 2.0, 1.0, false), 2.0);
        chk!(bad, dbeta(1.0, 2.0, 1.0, true), 0.69314718055994529);
        chk!(bad, dbeta(0.5, 0.5, 0.5, false), 0.63661977236758116);
        chk!(bad, dbeta(0.3, 50.0, 80.0, false), 1.2667585451569421);
        chk!(bad, dbeta(0.3, 50.0, 80.0, true), 0.2364613110838647);
        chk!(bad, dbeta(0.001, 2.5, 10000.0, false), 10.758754475772236);
        chk!(bad, dbeta(0.5, f64::INFINITY, f64::INFINITY, false), f64::INFINITY);
        chk!(bad, dbeta(0.2, 0.0, 3.0, false), 0.0);
        assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
    }

    #[test]
    fn qbeta_values() {
        let mut bad: Vec<String> = Vec::new();
        chk!(bad, qbeta(0.3, 2.0, 5.0, true, false), 0.18180347131894911);
        chk!(bad, qbeta(0.95, 0.5, 0.5, true, false), 0.99384417029756889);
        chk!(bad, qbeta(0.01, 10.0, 3.0, true, false), 0.46265708568307529);
        chk!(bad, qbeta(1e-10, 2.0, 3.0, true, false), 4.08249401580833e-06);
        chk!(bad, qbeta(-50.0, 2.0, 3.0, true, true), 5.6697293409510895e-12);
        chk!(bad, qbeta(-50.0, 2.0, 3.0, false, true), 0.99999996360246479);
        chk!(bad, qbeta(0.1, 2.0, 3.0, false, false), 0.6795394162781816);
        chk!(bad, qbeta(0.2066, 0.143891, 0.05, true, false), 0.17079623740412836);
        chk!(bad, qbeta(3.1554436208840472e-30, 0.125, 1.2621774483536189e-29, true, false), 9.0949470177219315e-13);
        chk!(bad, qbeta(0.95, 1e-09, 20.0, true, false), 5.5626846462680035e-309);
        chk!(bad, qbeta(0.5, 1e+05, 1e+05, true, false), 0.5);
        chk!(bad, qbeta(0.21, 0.001, 0.05, true, false), 2.3380802297871304e-302);
        chk!(bad, qbeta(0.7, 0.3, 4.0, true, false), 0.067658058003788435);
        chk!(bad, qbeta(0.999, 30.0, 0.2, true, false), 1.0);
        chk!(bad, qbeta(1e-05, 200.0, 300.0, false, false), 0.49492227652089194);
        chk!(bad, qbeta(-1e-10, 0.2, 0.03, true, true), 1.0);
        chk!(bad, qbeta(0.5, 0.0, 0.0, true, false), 0.5);
        chk!(bad, qbeta(0.3, 0.0, 2.0, true, false), 0.0);
        chk!(bad, qbeta(0.3, f64::INFINITY, f64::INFINITY, true, false), 0.5);
        chk!(bad, qbeta(1.5, 2.0, 3.0, true, false), f64::NAN);
        chk!(bad, qbeta(0.4, 1.5, 1.5, true, false), 0.42113190309999204);
        chk!(bad, qbeta(0.025, 15.5, 4.25, true, false), 0.58427989406334424);
        assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
    }

    // Warning texts as GNU R 4.5.0 emits them.
    #[test]
    fn beta_warnings() {
        take_warnings();
        assert_eq!(pbeta(1e-320, 1e-5, 0.5, true, false), 0.99995077031517055);
        assert_eq!(
            take_warnings(),
            vec!["bgrat(a=20.5, b=1e-05, x=1, y=9.99989e-321): z=1.99998e-319, b*z == 0 underflow, hence inaccurate pbeta()"]
        );
        assert_eq!(pbeta(0.1, 20.0, 100000.0, false, true), f64::NEG_INFINITY);
        assert_eq!(
            take_warnings(),
            vec!["pbeta(*, log.p=TRUE) -> bpser(a=100000, b=20, x=0.9,...) underflow to -Inf"]
        );
        assert_eq!(qbeta(0.05, 1.0, 0.001, true, false), 0.99999999999999989);
        assert_eq!(
            take_warnings(),
            vec!["qbeta(a, *) =: x0 with |pbeta(x0,*) - alpha| = 0.014598 is not accurate"]
        );
    }
}
