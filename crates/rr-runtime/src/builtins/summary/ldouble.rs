//! Software emulation of the x87 80-bit `long double` (R's `LDOUBLE` on
//! x86_64), used where GNU R accumulates in extended precision: `sum`,
//! `prod`, `mean`, `cumsum`, `cumprod`.
//!
//! Only what those accumulators need is provided: +, -, *, / with
//! round-to-nearest-even on a 64-bit significand, conversion from `f64`
//! and integers, and conversion back to `f64` (with double rounding, as
//! the hardware does). NaN propagation follows the x87 rules: a NaN
//! operand is returned quieted, and of two NaNs the one with the larger
//! significand wins (so `NA` beats R's default `NaN`).

#[derive(Clone, Copy, Debug, PartialEq)]
enum Class {
    Zero,
    Finite,
    Inf,
    NaN,
}

/// An x87 extended-precision value.
#[derive(Clone, Copy, Debug)]
pub struct F80 {
    class: Class,
    neg: bool,
    /// Unbiased exponent of the integer bit (bit 63 of `mant`).
    exp: i32,
    /// Significand with explicit integer bit. For NaNs, the x87 payload.
    mant: u64,
}

const EMAX: i32 = 16383;
const EMIN: i32 = -16382;
const QUIET: u64 = 1 << 62;
const INT_BIT: u64 = 1 << 63;

/// Shift right, OR-ing lost bits into the least significant bit.
fn shr_sticky(x: u128, n: u32) -> u128 {
    if n == 0 {
        x
    } else if n >= 128 {
        (x != 0) as u128
    } else {
        let lost = x & ((1u128 << n) - 1);
        (x >> n) | (lost != 0) as u128
    }
}

impl F80 {
    pub const ZERO: F80 = F80 { class: Class::Zero, neg: false, exp: 0, mant: 0 };
    pub const ONE: F80 = F80 { class: Class::Finite, neg: false, exp: 0, mant: INT_BIT };

    fn inf(neg: bool) -> F80 {
        F80 { class: Class::Inf, neg, exp: 0, mant: INT_BIT }
    }

    /// The x87 "real indefinite" produced by invalid operations.
    fn default_nan() -> F80 {
        F80 { class: Class::NaN, neg: true, exp: 0, mant: INT_BIT | QUIET }
    }

    pub fn is_nan(&self) -> bool {
        self.class == Class::NaN
    }

    pub fn is_finite(&self) -> bool {
        matches!(self.class, Class::Zero | Class::Finite)
    }

    pub fn from_f64(x: f64) -> F80 {
        let bits = x.to_bits();
        let neg = bits >> 63 != 0;
        let bexp = ((bits >> 52) & 0x7ff) as i32;
        let frac = bits & ((1u64 << 52) - 1);
        if bexp == 0x7ff {
            if frac == 0 {
                return F80::inf(neg);
            }
            // FLD quiets signaling NaNs.
            return F80 { class: Class::NaN, neg, exp: 0, mant: INT_BIT | QUIET | (frac << 11) };
        }
        if bexp == 0 {
            if frac == 0 {
                return F80 { class: Class::Zero, neg, exp: 0, mant: 0 };
            }
            // Subnormal double: normal in extended precision.
            let lz = frac.leading_zeros();
            let mant = frac << lz;
            let exp = -1022 - 52 + (63 - lz as i32);
            return F80 { class: Class::Finite, neg, exp, mant };
        }
        F80 { class: Class::Finite, neg, exp: bexp - 1023, mant: INT_BIT | (frac << 11) }
    }

    pub fn from_i64(x: i64) -> F80 {
        if x == 0 {
            return F80::ZERO;
        }
        let neg = x < 0;
        let m = x.unsigned_abs();
        let lz = m.leading_zeros();
        F80 { class: Class::Finite, neg, exp: 63 - lz as i32, mant: m << lz }
    }

    /// Round `sig * 2^(e127 - 127)` to extended precision.
    fn round_pack(neg: bool, mut e127: i32, mut sig: u128) -> F80 {
        if sig == 0 {
            return F80 { class: Class::Zero, neg, exp: 0, mant: 0 };
        }
        let lz = sig.leading_zeros();
        sig <<= lz;
        e127 -= lz as i32;
        // Denormal range: shift so the exponent is EMIN.
        if e127 < EMIN {
            sig = shr_sticky(sig, (EMIN - e127) as u32);
            e127 = EMIN;
        }
        let hi = (sig >> 64) as u64;
        let lo = sig as u64;
        let half = 1u64 << 63;
        let round_up = lo > half || (lo == half && hi & 1 == 1);
        let (mut mant, carry) = if round_up { hi.overflowing_add(1) } else { (hi, false) };
        if carry {
            mant = INT_BIT;
            e127 += 1;
        }
        if mant == 0 {
            return F80 { class: Class::Zero, neg, exp: 0, mant: 0 };
        }
        if e127 > EMAX {
            return F80::inf(neg);
        }
        F80 { class: Class::Finite, neg, exp: e127, mant }
    }

    fn nan_pick(a: F80, b: F80) -> F80 {
        match (a.is_nan(), b.is_nan()) {
            (true, true) => {
                let (ma, mb) = (a.mant | QUIET, b.mant | QUIET);
                if mb > ma {
                    F80 { mant: mb, ..b }
                } else {
                    F80 { mant: ma, ..a }
                }
            }
            (true, false) => F80 { mant: a.mant | QUIET, ..a },
            _ => F80 { mant: b.mant | QUIET, ..b },
        }
    }

    pub fn neg(self) -> F80 {
        F80 { neg: !self.neg, ..self }
    }

    pub fn add(self, b: F80) -> F80 {
        let a = self;
        if a.is_nan() || b.is_nan() {
            return F80::nan_pick(a, b);
        }
        match (a.class, b.class) {
            (Class::Inf, Class::Inf) => {
                return if a.neg == b.neg { a } else { F80::default_nan() };
            }
            (Class::Inf, _) => return a,
            (_, Class::Inf) => return b,
            (Class::Zero, Class::Zero) => {
                return F80 { neg: a.neg && b.neg, ..F80::ZERO };
            }
            (Class::Zero, _) => return b,
            (_, Class::Zero) => return a,
            _ => {}
        }
        // Both finite non-zero. Leading bit at position 126 leaves room
        // for the carry of an addition.
        let (big, small) = if (a.exp, a.mant) >= (b.exp, b.mant) { (a, b) } else { (b, a) };
        let sb = (big.mant as u128) << 63;
        let ss = shr_sticky((small.mant as u128) << 63, (big.exp - small.exp) as u32);
        let e127 = big.exp + 1;
        if big.neg == small.neg {
            F80::round_pack(big.neg, e127, sb + ss)
        } else {
            let d = sb - ss;
            if d == 0 {
                return F80::ZERO;
            }
            F80::round_pack(big.neg, e127, d)
        }
    }

    pub fn sub(self, b: F80) -> F80 {
        if b.is_nan() {
            return F80::nan_pick(self, b);
        }
        self.add(b.neg())
    }

    pub fn mul(self, b: F80) -> F80 {
        let a = self;
        if a.is_nan() || b.is_nan() {
            return F80::nan_pick(a, b);
        }
        let neg = a.neg != b.neg;
        match (a.class, b.class) {
            (Class::Inf, Class::Zero) | (Class::Zero, Class::Inf) => return F80::default_nan(),
            (Class::Inf, _) | (_, Class::Inf) => return F80::inf(neg),
            (Class::Zero, _) | (_, Class::Zero) => return F80 { neg, ..F80::ZERO },
            _ => {}
        }
        let p = (a.mant as u128) * (b.mant as u128);
        F80::round_pack(neg, a.exp + b.exp + 1, p)
    }

    pub fn div(self, b: F80) -> F80 {
        let a = self;
        if a.is_nan() || b.is_nan() {
            return F80::nan_pick(a, b);
        }
        let neg = a.neg != b.neg;
        match (a.class, b.class) {
            (Class::Inf, Class::Inf) | (Class::Zero, Class::Zero) => return F80::default_nan(),
            (Class::Inf, _) => return F80::inf(neg),
            (_, Class::Inf) => return F80 { neg, ..F80::ZERO },
            (Class::Zero, _) => return F80 { neg, ..F80::ZERO },
            (_, Class::Zero) => return F80::inf(neg),
            _ => {}
        }
        // Normalise denormal significands so both have the top bit set.
        let (ma, ea) = norm(a.mant, a.exp);
        let (mb, eb) = norm(b.mant, b.exp);
        let num = (ma as u128) << 64;
        let q1 = num / mb as u128;
        let r1 = num % mb as u128;
        let num2 = r1 << 64;
        let q2 = num2 / mb as u128;
        let r2 = num2 % mb as u128;
        let sig = (q1 << 62) | (q2 >> 2) | ((q2 & 3 != 0 || r2 != 0) as u128);
        F80::round_pack(neg, ea - eb + 1, sig)
    }

    /// `(double) x`: round to nearest double.
    pub fn to_f64(self) -> f64 {
        let sign = (self.neg as u64) << 63;
        match self.class {
            Class::Zero => f64::from_bits(sign),
            Class::Inf => f64::from_bits(sign | 0x7ff0_0000_0000_0000),
            Class::NaN => {
                let frac = (self.mant >> 11) & ((1u64 << 52) - 1);
                f64::from_bits(sign | 0x7ff0_0000_0000_0000 | frac | (1u64 << 51))
            }
            Class::Finite => {
                let (m, e) = norm(self.mant, self.exp);
                // Target: 53-bit significand, exponent >= -1022 (normal)
                // or denormal grid 2^-1074.
                let mut e = e;
                let mut sig = (m as u128) << 64;
                if e < -1022 {
                    sig = shr_sticky(sig, (-1022 - e) as u32);
                    e = -1022;
                }
                // keep top 53 bits of the 128
                let keep = (sig >> 75) as u64;
                let rest = sig & ((1u128 << 75) - 1);
                let half = 1u128 << 74;
                let up = rest > half || (rest == half && keep & 1 == 1);
                let mut mant = keep + up as u64;
                if mant == 1u64 << 53 {
                    mant >>= 1;
                    e += 1;
                }
                if e > 1023 {
                    return f64::from_bits(sign | 0x7ff0_0000_0000_0000);
                }
                if mant & (1u64 << 52) == 0 {
                    // denormal (or zero)
                    return f64::from_bits(sign | mant);
                }
                let bexp = (e + 1023) as u64;
                f64::from_bits(sign | (bexp << 52) | (mant & ((1u64 << 52) - 1)))
            }
        }
    }

    /// `x > DBL_MAX` for the accumulators' overflow checks.
    pub fn gt_dbl_max(&self) -> bool {
        match self.class {
            Class::Inf => !self.neg,
            Class::Finite => !self.neg && F80::from_f64(f64::MAX).lt_abs(self),
            _ => false,
        }
    }

    /// `x < -DBL_MAX`.
    pub fn lt_neg_dbl_max(&self) -> bool {
        match self.class {
            Class::Inf => self.neg,
            Class::Finite => self.neg && F80::from_f64(f64::MAX).lt_abs(self),
            _ => false,
        }
    }

    fn lt_abs(&self, other: &F80) -> bool {
        let (ma, ea) = norm(self.mant, self.exp);
        let (mb, eb) = norm(other.mant, other.exp);
        (ea, ma) < (eb, mb)
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

fn norm(m: u64, e: i32) -> (u64, i32) {
    if m == 0 || m & INT_BIT != 0 {
        (m, e)
    } else {
        let lz = m.leading_zeros();
        (m << lz, e - lz as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::F80;

    fn sum(xs: &[f64]) -> f64 {
        xs.iter().fold(F80::ZERO, |s, &x| s.add(F80::from_f64(x))).to_f64()
    }

    #[test]
    fn roundtrip() {
        for &x in &[0.0, -0.0, 1.0, 0.1, -3.5, 1e300, 5e-324, 2.2250738585072014e-308, f64::MAX] {
            assert_eq!(F80::from_f64(x).to_f64().to_bits(), x.to_bits());
        }
    }

    #[test]
    fn extended_sums() {
        // 1 + 2^-60 - 1 is exact in 64-bit precision but not in double.
        let r = F80::from_f64(1.0).add(F80::from_f64(2f64.powi(-60))).sub(F80::from_f64(1.0));
        assert_eq!(r.to_f64(), 2f64.powi(-60));
        assert_eq!(sum(&[0.1, 0.2, 0.3]), 0.6);
        assert_eq!(sum(&[1e308, 1e308, -1e308]), 1e308);
    }

    #[test]
    fn products_and_division() {
        let third = F80::from_f64(1.0).div(F80::from_f64(3.0));
        assert_eq!(third.to_f64(), 1.0 / 3.0);
        let p = F80::from_f64(0.1).mul(F80::from_f64(0.1));
        assert_eq!(p.to_f64(), 0.010000000000000002);
        assert_eq!(F80::from_f64(7.0).div(F80::from_i64(2)).to_f64(), 3.5);
    }

    #[test]
    fn nan_rules() {
        let na = F80::from_f64(crate::value::NA_REAL);
        let nan = F80::from_f64(f64::NAN);
        assert!(crate::value::is_na_real(nan.add(na).to_f64()));
        assert!(crate::value::is_na_real(na.add(nan).to_f64()));
        assert!(!crate::value::is_na_real(nan.add(F80::ONE).to_f64()));
    }
}
