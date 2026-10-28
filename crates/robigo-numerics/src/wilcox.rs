//! The Wilcoxon rank sum distribution: wilcox.c.

use super::gamma::{choose, lchoose};
use super::*;
use std::collections::HashMap;

/// `cwilcox()`'s memo table (`w[i][j][k]` in C): the number of choices
/// with statistic = k.
#[derive(Default)]
struct Cwilcox {
    memo: HashMap<(i32, i32, i32), f64>,
}

impl Cwilcox {
    /// R: nmath/wilcox.c:cwilcox
    fn get(&mut self, k: i32, m: i32, n: i32) -> f64 {
        let u = m * n;
        if k < 0 || k > u {
            return 0.0;
        }
        let c = u / 2;
        let mut k = k;
        if k > c {
            k = u - k; // hence k <= floor(u / 2)
        }
        let (i, j) = if m < n { (m, n) } else { (n, m) }; // hence i <= j

        if j == 0 {
            // and hence i == 0
            return if k == 0 { 1.0 } else { 0.0 };
        }

        // We can simplify things if k is small.  Consider the Mann-Whitney
        // definition, and sort y.  Then if the statistic is k, no more
        // than k of the y's can be <= any x[i], and since they are sorted
        // these can only be in the first k.  So the count is the same as
        // if there were just k y's.
        if j > 0 && k < j {
            return self.get(k, i, k);
        }

        if let Some(&v) = self.memo.get(&(i, j, k)) {
            return v;
        }
        let v = self.get(k - j, i - 1, j) + self.get(k, i, j - 1);
        self.memo.insert((i, j, k), v);
        v
    }
}

/// R: nmath/wilcox.c:dwilcox
pub fn dwilcox(x: f64, m: f64, n: f64, give_log: bool) -> f64 {
    if x.is_nan() || m.is_nan() || n.is_nan() {
        return x + m + n;
    }
    let m = r_forceint(m);
    let n = r_forceint(n);
    if m <= 0.0 || n <= 0.0 {
        return ML_NAN;
    }

    if r_nonint(x) {
        return r_d__0(give_log);
    }
    let x = r_forceint(x);
    if x < 0.0 || x > m * n {
        return r_d__0(give_log);
    }

    let (mm, nn, xx) = (m as i32, n as i32, x as i32);
    let mut w = Cwilcox::default();
    if give_log { w.get(xx, mm, nn).ln() - lchoose(m + n, n) } else { w.get(xx, mm, nn) / choose(m + n, n) }
}

/// R: nmath/wilcox.c:pwilcox
pub fn pwilcox(q: f64, m: f64, n: f64, lower_tail: bool, log_p: bool) -> f64 {
    if q.is_nan() || m.is_nan() || n.is_nan() {
        return q + m + n;
    }
    if !m.is_finite() || !n.is_finite() {
        return ML_NAN;
    }
    let m = r_forceint(m);
    let n = r_forceint(n);
    if m <= 0.0 || n <= 0.0 {
        return ML_NAN;
    }

    let mut q = (q + 1e-7).floor();

    if q < 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    if q >= m * n {
        return r_dt_1(lower_tail, log_p);
    }

    let (mm, nn) = (m as i32, n as i32);
    let mut w = Cwilcox::default();
    let c = choose(m + n, n);
    let mut p = 0.0;
    let mut lower_tail = lower_tail;
    // Use summation of probs over the shorter range
    if q <= m * n / 2.0 {
        let mut i = 0.0;
        while i <= q {
            p += w.get(i as i32, mm, nn) / c;
            i += 1.0;
        }
    } else {
        q = m * n - q;
        let mut i = 0.0;
        while i < q {
            p += w.get(i as i32, mm, nn) / c;
            i += 1.0;
        }
        lower_tail = !lower_tail; // p = 1 - p;
    }

    r_dt_val(p, lower_tail, log_p)
}

/// R: nmath/wilcox.c:qwilcox
pub fn qwilcox(x: f64, m: f64, n: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || m.is_nan() || n.is_nan() {
        return x + m + n;
    }
    if !x.is_finite() || !m.is_finite() || !n.is_finite() {
        return ML_NAN;
    }
    if r_q_p01_check(x, log_p) {
        return ML_NAN;
    }

    let m = r_forceint(m);
    let n = r_forceint(n);
    if m <= 0.0 || n <= 0.0 {
        return ML_NAN;
    }

    if x == r_dt_0(lower_tail, log_p) {
        return 0.0;
    }
    if x == r_dt_1(lower_tail, log_p) {
        return m * n;
    }

    let mut x = x;
    if log_p || !lower_tail {
        x = r_dt_qiv(x, lower_tail, log_p); // lower_tail, non-log "p"
    }

    let (mm, nn) = (m as i32, n as i32);
    let mut w = Cwilcox::default();
    let c = choose(m + n, n);
    let mut p = 0.0;
    let mut q = 0;
    if x <= 0.5 {
        x -= 10.0 * DBL_EPSILON;
        loop {
            p += w.get(q, mm, nn) / c;
            if p >= x {
                break;
            }
            q += 1;
        }
    } else {
        x = 1.0 - x + 10.0 * DBL_EPSILON;
        loop {
            p += w.get(q, mm, nn) / c;
            if p > x {
                q = (m * n - q as f64) as i32;
                break;
            }
            q += 1;
        }
    }

    q as f64
}
