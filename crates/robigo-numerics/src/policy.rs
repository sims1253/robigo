//! Runtime numeric policy for reductions. Exact is the default.
//!
//! Select a policy once at an operation's entry point, then call a generic
//! implementation with [`LDouble`] (exact) or [`f64`] (fast) as its
//! [`Accumulator`]. This monomorphizes the loops: there is no mode check per
//! element. Keep the accumulator type until the final conversion, including
//! across chunks and missing-value runs. In particular, an exact row-sum
//! buffer must be `Vec<LDouble>`, never a repeatedly rounded `Vec<f64>`.
//!
//! The runtime owns the setting per interpreter; this crate has no global
//! mode or environment access. Compilers that specialize these operations
//! must include [`NumericPolicy`] in their code cache key. The current VM
//! calls runtime builtins, so it does not bake the policy into cached code.
//!
//! ```
//! use robigo_numerics::policy::{sum, Accumulator, NumericPolicy};
//! use robigo_numerics::ldouble::LDouble;
//! fn chunks<A: Accumulator>(xs: &[f64], ys: &[f64]) -> f64 {
//!     A::ZERO.sum_from(xs).sum_from(ys).to_f64()
//! }
//! assert_eq!(chunks::<LDouble>(&[1e16, 1.0], &[-1e16]), 1.0);
//! let _ = sum(NumericPolicy::Fast, &[0.1, 0.2, 0.3]);
//! ```
//!
//! # Fast-mode error bounds
//!
//! Fast uses binary64 and four independent lanes for reductions (LLVM SLP
//! vectorizes the lanes on SIMD targets). Prefix scans retain sequential
//! dependencies. No target-specific instructions or unstable features are
//! required; the same Rust loops are the scalar fallback.
//!
//! Bounds below compare fast with exact for finite inputs, round-to-nearest,
//! and no overflow or underflow in either computation. Let `u = 2^-53`,
//! `g(k) = k*u/(1-k*u)`, `n` be the number of inputs, and `B` the scale:
//!
//! | Operation | Absolute error bound | B |
//! |---|---|---|
//! | sum, cumsum prefix | `g(2*n+8)*B` | `abs(init) + sum(abs(x))` |
//! | sum_dev | `g(3*n+8)*B` | `sum(abs(x)+abs(m))` |
//! | sum_sq_dev | `g(4*n+8)*B` | `sum((abs(x)+abs(m))^2)` |
//! | sum_cross_dev | `g(5*n+8)*B` | `sum((abs(x)+abs(xm))*(abs(y)+abs(ym)))` |
//! | mean (with correction) | `g(8*n+32)*B` | `sum(abs(x))/n` |
//! | prod, cumprod prefix | `g(2*n+8)*abs(exact)` | relative bound `g(2*n+8)` |
//! | add_each, after n columns | same as sum, per row | same as sum |
//!
//! For a nonzero exact result, divide the absolute bound by `abs(exact)`
//! for the relative bound. Cancellation can make that bound arbitrarily
//! large; there is no uniform relative guarantee for sums or covariance.
//! These conservative bounds include the final binary64 rounding of exact.
//! Empty means are NaN. Fast has binary64's exponent range: overflow,
//! underflow, reassociation of infinities, NaN payloads, and signed zero can
//! differ from exact. R-level missing-value handling remains in callers.

use crate::ldouble::LDouble;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub};

mod fast;

/// Interpreter-owned mode, also suitable as a compiler cache-key component.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum NumericPolicy {
    /// GNU R's sequential 80-bit accumulation, on every target.
    #[default]
    Exact,
    /// Binary64 accumulation, SIMD and reassociation allowed.
    Fast,
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::LDouble {}
    impl Sealed for f64 {}
}

/// Arithmetic and slice kernels for one statically selected policy.
///
/// Implemented only for `LDouble` and `f64`. Callers match `NumericPolicy`
/// before entering a generic operation. Scalar arithmetic uses the same
/// precision as the slice kernels, so correction passes and division do not
/// accidentally round exact accumulators early. Slice pairs must have equal
/// lengths; output slices must hold at least the input length.
pub trait Accumulator:
    sealed::Sealed
    + Copy
    + PartialOrd
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + AddAssign
    + AddAssign<f64>
    + MulAssign
    + DivAssign
{
    const ZERO: Self;
    const ONE: Self;
    fn from_f64(x: f64) -> Self;
    fn from_i64(x: i64) -> Self;
    fn to_f64(self) -> f64;
    fn sqrt(self) -> Self;
    fn sum_from(self, xs: &[f64]) -> Self;
    fn prod_from(self, xs: &[f64]) -> Self;
    fn cumsum_from(self, xs: &[f64], out: &mut [f64]) -> Self;
    fn cumprod_from(self, xs: &[f64], out: &mut [f64]) -> Self;
    fn sum_dev(self, xs: &[f64]) -> Self;
    fn sum_sq_dev(self, xs: &[f64]) -> Self;
    fn sum_cross_dev(self, xs: &[f64], ym: Self, ys: &[f64]) -> Self;
    fn add_each(acc: &mut [Self], xs: &[f64]);

    fn is_nan(self) -> bool {
        self != self
    }
    fn is_finite(self) -> bool {
        !(self - self).is_nan()
    }
    fn to_f64_clamped(self) -> f64 {
        if self > Self::from_f64(f64::MAX) {
            f64::INFINITY
        } else if self < Self::from_f64(-f64::MAX) {
            f64::NEG_INFINITY
        } else {
            self.to_f64()
        }
    }
}

impl Accumulator for LDouble {
    const ZERO: Self = LDouble::ZERO;
    const ONE: Self = LDouble::ONE;
    #[inline]
    fn from_f64(x: f64) -> Self {
        Self::from_f64(x)
    }
    #[inline]
    fn from_i64(x: i64) -> Self {
        Self::from_i64(x)
    }
    #[inline]
    fn to_f64(self) -> f64 {
        self.to_f64()
    }
    #[inline]
    fn sqrt(self) -> Self {
        self.sqrt()
    }
    #[inline]
    fn sum_from(self, xs: &[f64]) -> Self {
        self.sum_from(xs)
    }
    #[inline]
    fn prod_from(self, xs: &[f64]) -> Self {
        self.prod_from(xs)
    }
    #[inline]
    fn cumsum_from(self, xs: &[f64], out: &mut [f64]) -> Self {
        assert!(out.len() >= xs.len());
        self.cumsum_from(xs, out)
    }
    #[inline]
    fn cumprod_from(self, xs: &[f64], out: &mut [f64]) -> Self {
        assert!(out.len() >= xs.len());
        self.cumprod_from(xs, out)
    }
    #[inline]
    fn sum_dev(self, xs: &[f64]) -> Self {
        self.sum_dev(xs)
    }
    #[inline]
    fn sum_sq_dev(self, xs: &[f64]) -> Self {
        self.sum_sq_dev(xs)
    }
    #[inline]
    fn sum_cross_dev(self, xs: &[f64], ym: Self, ys: &[f64]) -> Self {
        assert_eq!(xs.len(), ys.len());
        self.sum_cross_dev(xs, ym, ys)
    }
    #[inline]
    fn add_each(acc: &mut [Self], xs: &[f64]) {
        assert!(acc.len() >= xs.len());
        Self::add_each(acc, xs)
    }
}

/// R: summary.c:real_mean. The overflow retry divides inputs in binary64;
/// the correction subtracts and divides in the accumulator's precision.
pub fn mean_with<A: Accumulator>(xs: &[f64]) -> f64 {
    let n = A::from_i64(xs.len() as i64);
    let mut s = A::ZERO.sum_from(xs);
    let finite_s = s.to_f64().is_finite();
    if finite_s {
        s /= n;
    } else {
        s = A::ZERO;
        for &v in xs {
            let q = if v.is_nan() { f64::from_bits(v.to_bits() | (1u64 << 51)) } else { v / xs.len() as f64 };
            s += A::from_f64(q);
        }
    }
    if finite_s && s.to_f64().is_finite() {
        s += s.sum_dev(xs) / n;
    } else if s.to_f64().is_finite() {
        let mut t = A::ZERO;
        for &v in xs {
            t += (A::from_f64(v) - s) / n;
        }
        s += t;
    }
    s.to_f64()
}

/// Sum with a single policy choice before the loop.
pub fn sum(policy: NumericPolicy, xs: &[f64]) -> f64 {
    match policy {
        NumericPolicy::Exact => LDouble::ZERO.sum_from(xs).to_f64(),
        NumericPolicy::Fast => 0.0.sum_from(xs),
    }
}
/// Product with a single policy choice before the loop.
pub fn prod(policy: NumericPolicy, xs: &[f64]) -> f64 {
    match policy {
        NumericPolicy::Exact => LDouble::ONE.prod_from(xs).to_f64(),
        NumericPolicy::Fast => 1.0.prod_from(xs),
    }
}
/// Corrected mean, including GNU R's overflow retry.
pub fn mean(policy: NumericPolicy, xs: &[f64]) -> f64 {
    match policy {
        NumericPolicy::Exact => mean_with::<LDouble>(xs),
        NumericPolicy::Fast => mean_with::<f64>(xs),
    }
}

#[cfg(test)]
mod tests;
