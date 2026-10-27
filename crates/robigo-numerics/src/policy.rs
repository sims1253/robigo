//! The numeric-policy seam ([docs/architecture.md](../../docs/architecture.md),
//! "Numerics"): one switch for every operation whose result depends on
//! the numeric mode.
//!
//! - **Exact** (the default): GNU R's algorithm bit for bit — sums,
//!   cumulative sums, sums of squared deviations, ... accumulated in
//!   80-bit long double exactly as R's C does on x86_64 (`LDouble`:
//!   x87 there, the software implementation elsewhere). This is what
//!   the conformance corpora verify.
//! - **Fast**: the same shapes in `f64` with four-way reassociated
//!   accumulation (auto-vectorizable; SIMD where the target has it).
//!   Results may differ from R in the last bits — allowed only when
//!   the user opts in at run time.
//!
//! The mode is a *runtime* setting (a relaxed atomic; reading it is a
//! plain load). Compiled code (the register VM, native code) must treat
//! it as part of the code cache key: code compiled under one policy
//! must not be reused under the other. Callers in the runtime should
//! branch once per vector, not per element.
//!
//! ```
//! use robigo_numerics::policy::{self, NumericPolicy};
//!
//! let xs = [0.1, 0.2, 0.3];
//! // exact: R's long-double sum, 0.1+0.2+0.3 == 0.6 exactly
//! assert_eq!(policy::sum(NumericPolicy::Exact, &xs), 0.6);
//! // fast: f64 reassociation — may differ in the last bit
//! let _ = policy::sum(NumericPolicy::Fast, &xs);
//! NumericPolicy::set_global(NumericPolicy::Fast);
//! assert_eq!(NumericPolicy::global(), NumericPolicy::Fast);
//! NumericPolicy::set_global(NumericPolicy::Exact);
//! ```

use crate::ldouble::LDouble;
use std::sync::atomic::{AtomicU8, Ordering};

/// The numeric mode. `Exact` is the default and the only mode the
/// conformance suites accept.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NumericPolicy {
    /// GNU R's bits: 80-bit long-double accumulation.
    Exact,
    /// `f64`, four-way reassociated (SIMD-friendly); last-bit
    /// differences allowed.
    Fast,
}

impl NumericPolicy {
    const EXACT: u8 = 0;
    const FAST: u8 = 1;

    /// The process-wide mode. Relaxed: this is a mode switch, not
    /// synchronization.
    pub fn global() -> NumericPolicy {
        match GLOBAL.load(Ordering::Relaxed) {
            NumericPolicy::FAST => NumericPolicy::Fast,
            _ => NumericPolicy::Exact,
        }
    }

    pub fn set_global(p: NumericPolicy) {
        GLOBAL.store(
            match p {
                NumericPolicy::Exact => NumericPolicy::EXACT,
                NumericPolicy::Fast => NumericPolicy::FAST,
            },
            Ordering::Relaxed,
        );
    }

    /// The policy to use when a caller does not choose explicitly: the
    /// global setting.
    #[inline]
    pub fn or_global(explicit: Option<NumericPolicy>) -> NumericPolicy {
        explicit.unwrap_or_else(NumericPolicy::global)
    }
}

static GLOBAL: AtomicU8 = AtomicU8::new(NumericPolicy::EXACT);

// ---------------------------------------------------------------------
// Kernels. Exact mirrors R's C loops (summary.c, cum.c, cov(): each
// step loads the double exactly and rounds once in long double); fast
// reassociates over four f64 accumulators.

/// `init + xs[0] + xs[1] + ...` — R's `sum()` (summary.c:medium_vecsum):
/// a long-double accumulator fed with exact double loads. Fast: four
/// f64 accumulators, tail folded in order.
pub fn sum(policy: NumericPolicy, xs: &[f64]) -> f64 {
    match policy {
        NumericPolicy::Exact => LDouble::ZERO.sum_from(xs).to_f64(),
        NumericPolicy::Fast => fast_sum(xs),
    }
}

/// `init + xs[0] + xs[1] + ...` with a running `init` (R's `sum(x, na.rm
/// = FALSE)` over chunks, or `sum(HEADER)` in the interpreter).
pub fn sum_from(policy: NumericPolicy, init: f64, xs: &[f64]) -> f64 {
    match policy {
        NumericPolicy::Exact => LDouble::from_f64(init).sum_from(xs).to_f64(),
        NumericPolicy::Fast => {
            let mut s = init;
            for k in (0..xs.len()).step_by(4) {
                let n = (xs.len() - k).min(4);
                s += xs[k..k + n].iter().sum::<f64>();
            }
            s
        }
    }
}

/// Four f64 accumulators over strided lanes (auto-vectorizes to SSE2/
/// AVX paired adds), remainder folded in order.
fn fast_sum(xs: &[f64]) -> f64 {
    let mut a = [0.0f64; 4];
    let mut chunks = xs.chunks_exact(4);
    for c in &mut chunks {
        a[0] += c[0];
        a[1] += c[1];
        a[2] += c[2];
        a[3] += c[3];
    }
    let mut s = (a[0] + a[1]) + (a[2] + a[3]);
    for &x in chunks.remainder() {
        s += x;
    }
    s
}

/// `s = init; out[i] = (double)(s += xs[i])` — R's `cumsum()` (cum.c).
pub fn cumsum(policy: NumericPolicy, init: f64, xs: &[f64], out: &mut [f64]) {
    assert!(out.len() >= xs.len());
    match policy {
        NumericPolicy::Exact => {
            LDouble::from_f64(init).cumsum_from(xs, out);
        }
        NumericPolicy::Fast => {
            let mut s = init;
            for (x, o) in xs.iter().zip(out.iter_mut()) {
                s += x;
                *o = s;
            }
        }
    }
}

/// `s = init; out[i] = (double)(s *= xs[i])` — R's `cumprod()` (cum.c).
pub fn cumprod(policy: NumericPolicy, init: f64, xs: &[f64], out: &mut [f64]) {
    assert!(out.len() >= xs.len());
    match policy {
        NumericPolicy::Exact => {
            LDouble::from_f64(init).cumprod_from(xs, out);
        }
        NumericPolicy::Fast => {
            let mut s = init;
            for (x, o) in xs.iter().zip(out.iter_mut()) {
                s *= x;
                *o = s;
            }
        }
    }
}

/// `init * xs[0] * xs[1] * ...` — R's `prod()`.
pub fn prod_from(policy: NumericPolicy, init: f64, xs: &[f64]) -> f64 {
    match policy {
        NumericPolicy::Exact => LDouble::from_f64(init).prod_from(xs).to_f64(),
        NumericPolicy::Fast => {
            let mut p = init;
            for k in (0..xs.len()).step_by(4) {
                let n = (xs.len() - k).min(4);
                p *= xs[k..k + n].iter().product::<f64>();
            }
            p
        }
    }
}

/// `Σ (x_i − m)²` in one pass — R's `var()` numerator (cov.c keeps a
/// long-double `s` exactly like this loop). Fast: two f64 accumulators
/// over element pairs (SSE2 `subpd/mulpd/addpd`).
pub fn sum_sq_dev(policy: NumericPolicy, m: f64, xs: &[f64]) -> f64 {
    match policy {
        NumericPolicy::Exact => LDouble::from_f64(m).sum_sq_dev(xs).to_f64(),
        NumericPolicy::Fast => {
            let mut s = [0.0f64; 2];
            let mut ch = xs.chunks_exact(2);
            for c in &mut ch {
                let d0 = c[0] - m;
                let d1 = c[1] - m;
                s[0] += d0 * d0;
                s[1] += d1 * d1;
            }
            for &x in ch.remainder() {
                let d = x - m;
                s[1] += d * d;
            }
            s[0] + s[1]
        }
    }
}

/// `Σ (x_i − xm)(y_i − ym)` — R's `cov()` numerator.
pub fn sum_cross_dev(policy: NumericPolicy, xm: f64, xs: &[f64], ym: f64, ys: &[f64]) -> f64 {
    let n = xs.len().min(ys.len());
    let (xs, ys) = (&xs[..n], &ys[..n]);
    match policy {
        NumericPolicy::Exact => LDouble::from_f64(xm).sum_cross_dev(xs, LDouble::from_f64(ym), ys).to_f64(),
        NumericPolicy::Fast => {
            let mut s = [0.0f64; 2];
            let mut ch = xs.chunks_exact(2).zip(ys.chunks_exact(2));
            for (x, y) in &mut ch {
                s[0] += (x[0] - xm) * (y[0] - ym);
                s[1] += (x[1] - xm) * (y[1] - ym);
            }
            let rx = xs.chunks_exact(2).remainder();
            let ry = ys.chunks_exact(2).remainder();
            for (x, y) in rx.iter().zip(ry) {
                s[1] += (x - xm) * (y - ym);
            }
            s[0] + s[1]
        }
    }
}

/// `acc[i] += xs[i]` for each i — R's `colSums()` accumulating column
/// sums through a long-double vector, one row at a time.
pub fn add_each(policy: NumericPolicy, acc: &mut [f64], xs: &[f64]) {
    assert!(acc.len() >= xs.len());
    match policy {
        NumericPolicy::Exact => {
            // Long-double column accumulators, exactly like colSums.
            // (The f64 `acc` view is only the final rounding's storage.)
            let mut ld: Vec<LDouble> = acc.iter().map(|&a| LDouble::from_f64(a)).collect();
            LDouble::add_each(&mut ld, xs);
            for (a, l) in acc.iter_mut().zip(ld) {
                *a = l.to_f64();
            }
        }
        NumericPolicy::Fast => {
            for (a, x) in acc.iter_mut().zip(xs) {
                *a += x;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_matches_r_sum() {
        // R: sum(c(0.1, 0.2, 0.3)) == 0.6 is TRUE (long double accumulator)
        let xs = [0.1, 0.2, 0.3];
        assert_eq!(sum(NumericPolicy::Exact, &xs), 0.6);
        assert_eq!(sum(NumericPolicy::Exact, &[1e16, 1.0, -1e16]), 1.0);
        // cumsum rounds each step to double, accumulated in long double
        // (GNU R: 0.3000000000000000444089, not the f64 sum 0.30000000000000004441
        // of the tie at the 17th digit — matches R's cum.c exactly)
        let mut out = [0.0; 3];
        cumsum(NumericPolicy::Exact, 0.0, &xs, &mut out);
        assert_eq!(out, [0.1, 0.30000000000000004, 0.59999999999999998]);
    }

    #[test]
    fn fast_is_close_and_policy_switches() {
        let xs: Vec<f64> = (0..1000).map(|i| (i as f64 * 0.37).sin()).collect();
        let e = sum(NumericPolicy::Exact, &xs);
        let f = sum(NumericPolicy::Fast, &xs);
        assert!((e - f).abs() < 1e-13, "e={e} f={f}");
        NumericPolicy::set_global(NumericPolicy::Fast);
        assert_eq!(NumericPolicy::global(), NumericPolicy::Fast);
        NumericPolicy::set_global(NumericPolicy::Exact);
        assert_eq!(NumericPolicy::or_global(None), NumericPolicy::Exact);
        assert_eq!(NumericPolicy::or_global(Some(NumericPolicy::Fast)), NumericPolicy::Fast);
    }
}
