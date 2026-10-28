//! The Wilcoxon signed rank distribution: signrank.c.

use super::*;

/// `w_init_maybe()` + `csignrank()`'s memo table (`w[]` in C) for one n.
struct CSignrank {
    n: i32,
    w: Vec<f64>,
    computed: bool,
}

impl CSignrank {
    /// R: nmath/signrank.c:w_init_maybe.  `None` means `calloc` failed; the
    /// requested element count is returned so the caller can raise R's
    /// error.
    fn new(n: f64) -> Result<Self, (u64, u64)> {
        let n = c_int_cast(n); // `(int) n`
        let u = n.wrapping_mul(n.wrapping_add(1)) / 2;
        let c = u / 2;
        // `calloc((size_t) c + 1, sizeof(double))`: a negative count wraps.
        let nelem = (c as i64).wrapping_add(1) as u64;
        let mut w: Vec<f64> = Vec::new();
        if nelem > usize::MAX as u64 || w.try_reserve_exact(nelem as usize).is_err() {
            return Err((nelem, 8));
        }
        w.resize(nelem as usize, 0.0);
        Ok(CSignrank { n, w, computed: false })
    }

    /// R: nmath/signrank.c:csignrank
    fn get(&mut self, k: i32) -> f64 {
        let u = self.n * (self.n + 1) / 2;
        let c = u / 2;

        if k < 0 || k > u {
            return 0.0;
        }
        let mut k = k;
        if k > c {
            k = u - k;
        }

        if self.n == 1 {
            return 1.0;
        }
        if !self.computed {
            self.w[0] = 1.0;
            self.w[1] = 1.0;
            for j in 2..self.n + 1 {
                let end = imin2(j * (j + 1) / 2, c);
                let mut i = end;
                while i >= j {
                    self.w[i as usize] += self.w[(i - j) as usize];
                    i -= 1;
                }
            }
            self.computed = true;
        }

        self.w[k as usize]
    }
}

/// R: nmath/signrank.c:dsignrank
pub fn dsignrank(x: f64, n: f64, give_log: bool) -> f64 {
    if x.is_nan() || n.is_nan() {
        return x + n;
    }
    let n = r_forceint(n);
    if n <= 0.0 {
        return ML_NAN;
    }

    if r_nonint(x) {
        return r_d__0(give_log);
    }
    let x = r_forceint(x);
    if x < 0.0 || x > n * (n + 1.0) / 2.0 {
        return r_d__0(give_log);
    }

    let mut w = match CSignrank::new(n) {
        Ok(w) => w,
        Err((nelem, elsize)) => return alloc_error(nelem, elsize),
    };
    r_d_exp(w.get(c_int_cast(x)).ln() - n * M_LN2, give_log)
}

/// R: nmath/signrank.c:psignrank
pub fn psignrank(x: f64, n: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || n.is_nan() {
        return x + n;
    }
    if !n.is_finite() {
        return ML_NAN;
    }
    let n = r_forceint(n);
    if n <= 0.0 {
        return ML_NAN;
    }

    let x = r_forceint(x + 1e-7);
    if x < 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    if x >= n * (n + 1.0) / 2.0 {
        return r_dt_1(lower_tail, log_p);
    }

    let mut w = match CSignrank::new(n) {
        Ok(w) => w,
        Err((nelem, elsize)) => return alloc_error(nelem, elsize),
    };
    let f = (-n * M_LN2).exp();
    let mut p = 0.0;
    let mut lower_tail = lower_tail;
    let mut x = x;
    if x <= n * (n + 1.0) / 4.0 {
        let mut i = 0.0;
        while i <= x {
            p += w.get(c_int_cast(i)) * f;
            i += 1.0;
        }
    } else {
        x = n * (n + 1.0) / 2.0 - x;
        let mut i = 0.0;
        while i < x {
            p += w.get(c_int_cast(i)) * f;
            i += 1.0;
        }
        lower_tail = !lower_tail; // p = 1 - p;
    }

    r_dt_val(p, lower_tail, log_p)
}

/// R: nmath/signrank.c:qsignrank
pub fn qsignrank(x: f64, n: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || n.is_nan() {
        return x + n;
    }
    if !x.is_finite() || !n.is_finite() {
        return ML_NAN;
    }
    if r_q_p01_check(x, log_p) {
        return ML_NAN;
    }

    let n = r_forceint(n);
    if n <= 0.0 {
        return ML_NAN;
    }

    if x == r_dt_0(lower_tail, log_p) {
        return 0.0;
    }
    if x == r_dt_1(lower_tail, log_p) {
        return n * (n + 1.0) / 2.0;
    }

    let mut x = x;
    if log_p || !lower_tail {
        x = r_dt_qiv(x, lower_tail, log_p); // lower_tail, non-log "p"
    }

    let mut w = match CSignrank::new(n) {
        Ok(w) => w,
        Err((nelem, elsize)) => return alloc_error(nelem, elsize),
    };
    let f = (-n * M_LN2).exp();
    let mut p = 0.0;
    let mut q = 0;
    if x <= 0.5 {
        x -= 10.0 * DBL_EPSILON;
        loop {
            p += w.get(q) * f;
            if p >= x {
                break;
            }
            q += 1;
        }
    } else {
        x = 1.0 - x + 10.0 * DBL_EPSILON;
        loop {
            p += w.get(q) * f;
            if p > x {
                q = c_int_cast(n * (n + 1.0) / 2.0 - q as f64);
                break;
            }
            q += 1;
        }
    }

    q as f64
}
