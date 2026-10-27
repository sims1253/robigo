//! R's `%%` and `%/%` on doubles: long double remainders as in
//! arithmetic.c.

use crate::ldouble::LDouble;

/// GNU R's `c_eps`: LDBL_EPSILON (2^-63) where long double is wider than
/// double, as on x86_64.
const C_EPS: f64 = 1.0842021724855044e-19;

/// Whether `myfmod(x1, x2)` warns "probable complete loss of accuracy in
/// modulus" (the quotient exceeds 1/LDBL_EPSILON); callers warn.
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
    let fq = LDouble::from_f64(q.floor());
    let tmp = LDouble::from_f64(x1) - fq * LDouble::from_f64(x2);
    (fq + floorl(tmp / LDouble::from_f64(x2))).to_f64()
}
