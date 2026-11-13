//! R's `%%` and `%/%` on doubles: long double remainders as in
//! arithmetic.c.

use crate::ldouble::LDouble;

/// GNU R's `c_eps`: LDBL_EPSILON (2^-63) where long double is wider than
/// double, as on x86_64.
const C_EPS: f64 = 1.0842021724855044e-19;

/// Whether `myfmod(x1, x2)` warns "probable complete loss of accuracy in
/// modulus" (the quotient exceeds 1/LDBL_EPSILON); callers warn. Inlined
/// into the VM's scalar `%%` (`vm::arith_fast`), whatever the caller's size.
#[inline(always)]
pub fn fmod_warns(x1: f64, x2: f64) -> bool {
    if x2 == 0.0 || (x2.abs() * C_EPS > 1.0 && x1.is_finite() && x1.abs() <= x2.abs()) {
        return false;
    }
    let q = x1 / x2;
    q.is_finite() && q.abs() * C_EPS > 1.0
}

/// `floorl`.
fn floorl(x: LDouble) -> LDouble {
    let r = x.rint();
    if r > x { r - 1.0 } else { r }
}

/// `x1 %% x2` (without its warning: see [`fmod_warns`]).
// R: arithmetic.c:myfmod
pub fn myfmod(x1: f64, x2: f64) -> f64 {
    if x2 == 0.0 {
        return f64::NAN;
    }
    if x2.abs() * C_EPS > 1.0 && x1.is_finite() && x1.abs() <= x2.abs() {
        return if x1.abs() == x2.abs() {
            0.0
        } else if (x1 < 0.0 && x2 > 0.0) || (x2 < 0.0 && x1 > 0.0) {
            x1 + x2
        } else {
            x1
        };
    }
    let q = x1 / x2;
    let x2l = LDouble::from_f64(x2);
    let tmp = LDouble::from_f64(x1) - LDouble::from_f64(q.floor()) * x2l;
    (tmp - floorl(tmp / x2l) * x2l).to_f64()
}

/// `x1 %/% x2`.
// R: arithmetic.c:myfloor
pub fn myfloor(x1: f64, x2: f64) -> f64 {
    let q = x1 / x2;
    if x2 == 0.0 || q.abs() * C_EPS > 1.0 || !q.is_finite() {
        return q;
    }
    if q.abs() < 1.0 {
        return if q < 0.0 || (x1 < 0.0 && x2 > 0.0) || (x1 > 0.0 && x2 < 0.0) { -1.0 } else { 0.0 };
    }
    // Integers within 2^53 (`%/%` of integer-valued doubles, the common
    // case): `floor(q)` is the exact quotient's floor (rounding `x1 / x2` up
    // to an integer would take |x1| > 2^53), so the long double remainder
    // below is exact with |tmp| < |x2| and the same sign, its quotient's
    // floor is 0 (or -0, which adds nothing to the nonzero `floor(q)`).
    if x1 == x1.trunc() && x2 == x2.trunc() && x1.abs() <= TWO_53 && x2.abs() <= TWO_53 {
        return q.floor();
    }
    myfloor_ld(x1, x2, q)
}

const TWO_53: f64 = 9007199254740992.0;

/// [`myfloor`]'s general case, in long double.
fn myfloor_ld(x1: f64, x2: f64, q: f64) -> f64 {
    let fq = LDouble::from_f64(q.floor());
    let tmp = LDouble::from_f64(x1) - fq * LDouble::from_f64(x2);
    (fq + floorl(tmp / LDouble::from_f64(x2))).to_f64()
}

#[cfg(test)]
mod myfloor_tests {
    use super::*;

    /// The integer fast path of `myfloor` against the long double formula.
    #[test]
    fn integer_fast_path_is_the_long_double_result() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut checked = 0;
        for k in 0..400_000 {
            // Magnitudes spread from 1 to 2^53, both signs.
            let a = (next() >> (11 + (k % 53))) as i64;
            let b = (next() >> (11 + ((k / 7) % 53))) as i64;
            let (x1, x2) = (if k & 1 == 0 { a } else { -a } as f64, if k & 2 == 0 { b } else { -b } as f64);
            if x2 == 0.0 {
                continue;
            }
            let q = x1 / x2;
            if q.abs() * C_EPS > 1.0 || q.abs() < 1.0 {
                continue;
            }
            assert_eq!(myfloor(x1, x2).to_bits(), myfloor_ld(x1, x2, q).to_bits(), "{} %/% {}", x1, x2);
            checked += 1;
        }
        assert!(checked > 100_000);
        for (x1, x2) in
            [(9007199254740992.0, 3.0), (-9007199254740991.0, 7.0), (2147483647.0, -2.0), (-5.0, 2.0), (6.0, -3.0)]
        {
            assert_eq!(myfloor(x1, x2).to_bits(), myfloor_ld(x1, x2, x1 / x2).to_bits());
        }
    }
}
