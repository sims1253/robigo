//! R's `LDOUBLE` accumulators for `sum`, `prod`, `mean`, `cumsum` and
//! `cumprod`: the x87 80-bit `long double` GNU R uses on x86_64 (see
//! [`crate::nmath::ldouble`]), with the operations those accumulators
//! need.

use crate::nmath::ldouble::LDouble;

/// An x87 extended-precision value.
#[derive(Clone, Copy, Debug)]
pub struct F80(LDouble);

impl F80 {
    pub const ZERO: F80 = F80(LDouble::ZERO);
    pub const ONE: F80 = F80(LDouble::ONE);

    pub fn is_nan(&self) -> bool {
        self.0.is_nan()
    }

    /// Neither infinite nor NaN.
    pub fn is_finite(&self) -> bool {
        let d = self.0 - self.0;
        !d.is_nan()
    }

    pub fn from_f64(x: f64) -> F80 {
        F80(LDouble::from_f64(x))
    }

    pub fn from_i64(x: i64) -> F80 {
        F80(LDouble::from_i64(x))
    }

    pub fn neg(self) -> F80 {
        F80(self.0 * -1.0)
    }

    #[inline]
    pub fn add(self, b: F80) -> F80 {
        F80(self.0 + b.0)
    }

    pub fn sub(self, b: F80) -> F80 {
        F80(self.0 - b.0)
    }

    pub fn mul(self, b: F80) -> F80 {
        F80(self.0 * b.0)
    }

    pub fn div(self, b: F80) -> F80 {
        F80(self.0 / b.0)
    }

    /// `self + xs[0] + ...` sequentially (R's `LDOUBLE s; s += x[i]`).
    pub fn sum_slice(self, xs: &[f64]) -> F80 {
        F80(self.0.sum_from(xs))
    }

    /// `self * xs[0] * ...` sequentially.
    pub fn prod_slice(self, xs: &[f64]) -> F80 {
        F80(self.0.prod_from(xs))
    }

    /// Running sums from `self` into `ys` (rounded to double).
    pub fn cumsum_slice(self, xs: &[f64], ys: &mut [f64]) -> F80 {
        F80(self.0.cumsum_from(xs, ys))
    }

    /// `sum((long double) x - self)` over `xs`.
    pub fn sum_dev(self, xs: &[f64]) -> F80 {
        F80(self.0.sum_dev(xs))
    }

    /// `(double) x`: round to nearest double.
    pub fn to_f64(self) -> f64 {
        self.0.to_f64()
    }

    /// `x > DBL_MAX` for the accumulators' overflow checks.
    pub fn gt_dbl_max(&self) -> bool {
        self.0 > LDouble::from_f64(f64::MAX)
    }

    /// `x < -DBL_MAX`.
    pub fn lt_neg_dbl_max(&self) -> bool {
        self.0 < LDouble::from_f64(-f64::MAX)
    }

    /// R's idiom for storing an accumulator: `s > DBL_MAX ? Inf : s < -DBL_MAX ? -Inf : (double) s`.
    pub fn to_f64_clamped(self) -> f64 {
        if self.gt_dbl_max() {
            f64::INFINITY
        } else if self.lt_neg_dbl_max() {
            f64::NEG_INFINITY
        } else {
            self.to_f64()
        }
    }
}
