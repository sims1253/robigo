//! The binomial distribution and its building blocks: bd0.c (bd0, ebd0),
//! dbinom.c, pbinom.c, qbinom.c and the discrete quantile search of
//! qDiscrete_search.h (shared with qpois).

use super::gamma::stirlerr;
use super::norm::qnorm;
use super::pgamma::log1pmx;
use super::*;

/// R: nmath/bd0.c:bd0 -- x * log(x/np) + np - x, computed stably
pub fn bd0(x: f64, np: f64) -> f64 {
    if !x.is_finite() || !np.is_finite() || np == 0.0 {
        return ML_NAN;
    }
    if (x - np).abs() < 0.1 * (x + np) {
        let mut v = (x - np) / (x + np);
        let mut s = (x - np) * v;
        if s.abs() < DBL_MIN {
            return s;
        }
        let mut ej = 2.0 * x * v;
        v *= v;
        for j in 1..1000 {
            ej *= v;
            let s_ = s;
            s += ej / ((j << 1) + 1) as f64;
            if s == s_ {
                return s;
            }
        }
        mathlib_warning(format!(
            "bd0({}, {}): T.series failed to converge in 1000 it.; s={}, ej/(2j+1)={}\n",
            fmt_g(x),
            fmt_g(np),
            fmt_g(s),
            fmt_g(ej / ((1000 << 1) + 1) as f64)
        ));
    }
    x * (x / np).ln() + np - x
}

include!("bd0_table.rs");

/// C `frexp`: x = r * 2^e with r in [0.5, 1).
fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // subnormal: scale up first
        let (r, e) = frexp(x * f64::from_bits(0x4350_0000_0000_0000)); // 2^54
        return (r, e - 54);
    }
    let e = exp - 1022;
    let r = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (r, e)
}

/// R: nmath/bd0.c:ebd0 -- bd0(x, M) as yh + yl.
pub fn ebd0(x: f64, m: f64) -> (f64, f64) {
    const SB: i32 = 10;
    const S: f64 = (1u32 << SB) as f64;
    const N: i32 = 128;

    let mut yl = 0.0;
    let mut yh = 0.0;

    if x == m {
        return (yh, yl);
    }
    if x == 0.0 {
        return (m, yl);
    }
    if m == 0.0 {
        return (ML_POSINF, yl);
    }
    if m / x == ML_POSINF {
        return (m, yl);
    }
    let (r, e) = frexp(m / x);
    if M_LN2 * (-e as f64) > 1.0 + DBL_MAX / x {
        return (ML_POSINF, yl);
    }
    let i = ((r - 0.5) * (2 * N) as f64 + 0.5).floor() as i32;
    let f = (S / (0.5 + i as f64 / (2.0 * N as f64)) + 0.5).floor();
    let fg = ldexp(f, -(e + SB));
    if fg == ML_POSINF {
        return (fg, yl);
    }
    let add1 = |d: f64, yh: &mut f64, yl: &mut f64| {
        let d1 = (d + 0.5).floor();
        let d2 = d - d1;
        *yh += d1;
        *yl += d2;
    };
    add1(-x * log1pmx((m * fg - x) / x), &mut yh, &mut yl);
    if fg == 1.0 {
        return (yh, yl);
    }
    let i = i as usize;
    for j in 0..4 {
        add1(x * BD0_SCALE[i][j], &mut yh, &mut yl);
        add1(-x * BD0_SCALE[0][j] * e as f64, &mut yh, &mut yl);
        if !yh.is_finite() {
            return (ML_POSINF, 0.0);
        }
    }
    add1(m, &mut yh, &mut yl);
    add1(-m * fg, &mut yh, &mut yl);
    (yh, yl)
}

/// R: nmath/dbinom.c:pow1p -- (1+x)^y, accurate for small |x|
pub fn pow1p(x: f64, y: f64) -> f64 {
    if y.is_nan() {
        return if x == 0.0 { 1.0 } else { y };
    }
    if 0.0 <= y && y == y.trunc() && y <= 4.0 {
        match y as i32 {
            0 => return 1.0,
            1 => return x + 1.0,
            2 => return x * (x + 2.0) + 1.0,
            3 => return x * (x * (x + 3.0) + 3.0) + 1.0,
            4 => return x * (x * (x * (x + 4.0) + 6.0) + 4.0) + 1.0,
            _ => {}
        }
    }
    if (x + 1.0) - 1.0 == x || x.abs() > 0.5 || x.is_nan() {
        (1.0 + x).powf(y)
    } else {
        (y * log1p(x)).exp()
    }
}

/// R: nmath/dbinom.c:dbinom_raw
pub fn dbinom_raw(x: f64, n: f64, p: f64, q: f64, give_log: bool) -> f64 {
    if p == 0.0 {
        return if x == 0.0 { r_d__1(give_log) } else { r_d__0(give_log) };
    }
    if q == 0.0 {
        return if x == n { r_d__1(give_log) } else { r_d__0(give_log) };
    }
    if x == 0.0 {
        if n == 0.0 {
            return r_d__1(give_log);
        }
        return if p > q {
            if give_log {
                n * q.ln()
            } else {
                q.powf(n)
            }
        } else if give_log {
            n * log1p(-p)
        } else {
            pow1p(-p, n)
        };
    }
    if x == n {
        return if p > q {
            if give_log {
                n * log1p(-q)
            } else {
                pow1p(-q, n)
            }
        } else if give_log {
            n * p.ln()
        } else {
            p.powf(n)
        };
    }
    if x < 0.0 || x > n {
        return r_d__0(give_log);
    }
    let lc = stirlerr(n) - stirlerr(x) - stirlerr(n - x) - bd0(x, n * p) - bd0(n - x, n * q);
    let lf = M_LN_2PI + x.ln() + log1p(-x / n);
    r_d_exp(lc - 0.5 * lf, give_log)
}

/// R: nmath/dbinom.c:dbinom
pub fn dbinom(x: f64, n: f64, p: f64, give_log: bool) -> f64 {
    if x.is_nan() || n.is_nan() || p.is_nan() {
        return x + n + p;
    }
    if !(0.0..=1.0).contains(&p) || r_d_neg_i_nonint(n) {
        return ML_NAN;
    }
    if let Some(v) = r_d_nonint_check(x, give_log) {
        return v;
    }
    if x < 0.0 || !x.is_finite() {
        return r_d__0(give_log);
    }
    let n = r_forceint(n);
    let x = r_forceint(x);
    dbinom_raw(x, n, p, 1.0 - p, give_log)
}

/// R: nmath/pbinom.c:pbinom
pub fn pbinom(x: f64, n: f64, p: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || n.is_nan() || p.is_nan() {
        return x + n + p;
    }
    if !n.is_finite() || !p.is_finite() {
        return ML_NAN;
    }
    if r_nonint(n) {
        mathlib_warning(format!("non-integer n = {}", fmt_f(n)));
        return ML_NAN;
    }
    let n = r_forceint(n);
    if n < 0.0 || !(0.0..=1.0).contains(&p) {
        return ML_NAN;
    }
    if x < 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    let x = (x + 1e-7).floor();
    if n <= x {
        return r_dt_1(lower_tail, log_p);
    }
    super::beta::pbeta(p, x + 1.0, n - x, !lower_tail, log_p)
}

/// R: nmath/qbinom.c:qbinom
pub fn qbinom(p: f64, n: f64, pr: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || n.is_nan() || pr.is_nan() {
        return p + n + pr;
    }
    if !n.is_finite() || !pr.is_finite() {
        return ML_NAN;
    }
    if !p.is_finite() && !log_p {
        return ML_NAN;
    }
    let n = r_forceint(n);
    if !(0.0..=1.0).contains(&pr) || n < 0.0 {
        return ML_NAN;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, n, lower_tail, log_p) {
        return v;
    }
    if pr == 0.0 || n == 0.0 {
        return 0.0;
    }
    if pr == 1.0 {
        return n;
    }
    let q = 1.0 - pr;
    let mu = n * pr;
    let sigma = (n * pr * q).sqrt();
    let gamma = (q - pr) / sigma;
    discrete_quantile(p, mu, sigma, gamma, Some(n), lower_tail, log_p, &|y, lt, lg| pbinom(y, n, pr, lt, lg))
}

type PDist<'a> = &'a dyn Fn(f64, bool, bool) -> f64;

/// R: nmath/qDiscrete_search.h:do_search
fn do_search(
    mut y: f64,
    z: &mut f64,
    p: f64,
    pdist: PDist,
    max_y: Option<f64>,
    incr: f64,
    lower_tail: bool,
    log_p: bool,
) -> f64 {
    let left = if lower_tail { *z >= p } else { *z < p };
    if left {
        loop {
            let mut newz = -1.0;
            if y > 0.0 {
                newz = pdist(y - incr, lower_tail, log_p);
            } else if y < 0.0 {
                y = 0.0;
            }
            if y == 0.0 || newz.is_nan() || (if lower_tail { newz < p } else { newz >= p }) {
                return y;
            }
            y = fmax2(0.0, y - incr);
            *z = newz;
        }
    } else {
        loop {
            let prevy = y;
            let mut newz = -1.0;
            y += incr;
            match max_y {
                Some(mx) => {
                    if y < mx {
                        newz = pdist(y, lower_tail, log_p);
                    } else if y > mx {
                        y = mx;
                    }
                }
                None => newz = pdist(y, lower_tail, log_p),
            }
            if max_y.is_some_and(|mx| y == mx) || newz.is_nan() || (if lower_tail { newz >= p } else { newz < p }) {
                if incr <= 1.0 {
                    *z = newz;
                    return y;
                }
                return prevy;
            }
            *z = newz;
        }
    }
}

/// R: nmath/qDiscrete_search.h:q_DISCRETE_BODY
pub fn discrete_quantile(
    p: f64,
    mu: f64,
    sigma: f64,
    gamma: f64,
    max_y: Option<f64>,
    lower_tail: bool,
    log_p: bool,
    pdist: PDist,
) -> f64 {
    let mut p = p;
    let mut z = qnorm(p, 0.0, 1.0, lower_tail, log_p);
    let mut y = r_forceint(mu + sigma * (z + gamma * (z * z - 1.0) / 6.0));
    match max_y {
        Some(mx) => {
            if y > mx {
                y = mx;
            } else if y < 0.0 {
                y = 0.0;
            }
        }
        None => {
            if y < 0.0 {
                y = 0.0;
            }
        }
    }
    z = pdist(y, lower_tail, log_p);

    const PF_N: f64 = 8.0;
    const PF_L: f64 = 2.0;
    const Y_LARGE: f64 = 4096.0;
    const INC_F: f64 = 1.0 / 64.0;
    const I_SHRINK: f64 = 8.0;
    const REL_TOL: f64 = 1e-15;
    const XF: f64 = 4.0;

    if log_p {
        let e = PF_L * DBL_EPSILON;
        if lower_tail && p > -DBL_MAX {
            p *= 1.0 + e;
        } else {
            p *= 1.0 - e;
        }
    } else {
        let e = PF_N * DBL_EPSILON;
        if lower_tail {
            p *= 1.0 - e;
        } else if 1.0 - p > XF * e {
            p *= 1.0 + e;
        }
    }
    if y < Y_LARGE {
        return do_search(y, &mut z, p, pdist, max_y, 1.0, lower_tail, log_p);
    }
    let mut incr = (y * INC_F).floor();
    loop {
        let oldincr = incr;
        y = do_search(y, &mut z, p, pdist, max_y, incr, lower_tail, log_p);
        incr = fmax2(1.0, (incr / I_SHRINK).floor());
        if !(oldincr > 1.0 && incr > y * REL_TOL) {
            break;
        }
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binom_values() {
        // GNU R 4.5.0, sprintf("%.17g", ...)
        assert_eq!(dbinom(3.0, 10.0, 0.3, false), 0.26682793199999993);
        assert_eq!(dbinom(500.0, 1000.0, 0.5, false), 0.025225018178360804);
        assert_eq!(frexp(10.0), (0.625, 4));
        assert_eq!(frexp(f64::from_bits(1)), (0.5, -1073));
    }
}
