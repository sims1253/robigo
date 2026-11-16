//! machar.c's `machar` (ACM algorithm 665, W. J. Cody): the parameters of
//! a floating-point type found by computing with it, as GNU R's
//! `Init_R_Machine` does for `double` (`machar`) and `long double`
//! (`machar_LD`) to make `.Machine`.
//!
//! C's `volatile` keeps each intermediate in the declared type; `f64` on
//! SSE and [`LDouble`]'s x87 operations round every result to their format
//! already, so the port needs no such device.

use crate::ldouble::LDouble;

/// The floating-point operations machar uses.
pub trait MachFloat: Copy + PartialEq + PartialOrd {
    fn from_i32(i: i32) -> Self;
    fn add(self, b: Self) -> Self;
    fn sub(self, b: Self) -> Self;
    fn mul(self, b: Self) -> Self;
    fn div(self, b: Self) -> Self;
    /// `fabs` / `fabsl`.
    fn abs(self) -> Self;
}

impl MachFloat for f64 {
    fn from_i32(i: i32) -> f64 {
        i as f64
    }
    fn add(self, b: f64) -> f64 {
        self + b
    }
    fn sub(self, b: f64) -> f64 {
        self - b
    }
    fn mul(self, b: f64) -> f64 {
        self * b
    }
    fn div(self, b: f64) -> f64 {
        self / b
    }
    fn abs(self) -> f64 {
        f64::abs(self)
    }
}

impl MachFloat for LDouble {
    fn from_i32(i: i32) -> LDouble {
        LDouble::from_i64(i as i64)
    }
    fn add(self, b: LDouble) -> LDouble {
        self + b
    }
    fn sub(self, b: LDouble) -> LDouble {
        self - b
    }
    fn mul(self, b: LDouble) -> LDouble {
        self * b
    }
    fn div(self, b: LDouble) -> LDouble {
        self / b
    }
    fn abs(self) -> LDouble {
        let zero = LDouble::from_i64(0);
        if self < zero { zero - self } else { self }
    }
}

/// What machar reports (machar.c's header gives each meaning).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Machar<F> {
    pub ibeta: i32,
    pub it: i32,
    pub irnd: i32,
    pub ngrd: i32,
    pub machep: i32,
    pub negep: i32,
    pub iexp: i32,
    pub minexp: i32,
    pub maxexp: i32,
    pub eps: F,
    pub epsneg: F,
    pub xmin: F,
    pub xmax: F,
}

/// `machar()` for the type `F`: FLT_RADIX is 2 (GNU R takes `ibeta` from
/// it rather than computing it).
// R: machar.c:MACH_NAME
pub fn machar<F: MachFloat>() -> Machar<F> {
    let one = F::from_i32(1);
    let two = one.add(one);
    let zero = one.sub(one);

    // determine ibeta, beta ala malcolm
    let mut a = one;
    loop {
        a = a.add(a);
        let temp = a.add(one);
        let temp1 = temp.sub(a);
        if temp1.sub(one) != zero {
            break;
        }
    }
    let ibeta = 2;
    let beta = F::from_i32(ibeta);

    // determine it, irnd
    let mut it = 0;
    let mut b = one;
    loop {
        it += 1;
        b = b.mul(beta);
        let temp = b.add(one);
        let temp1 = temp.sub(b);
        if temp1.sub(one) != zero {
            break;
        }
    }
    let mut irnd = 0;
    let betah = beta.div(two);
    let temp = a.add(betah);
    if temp.sub(a) != zero {
        irnd = 1;
    }
    let tempa = a.add(beta);
    let temp = tempa.add(betah);
    if irnd == 0 && temp.sub(tempa) != zero {
        irnd = 2;
    }

    // determine negep, epsneg
    let mut negep = it + 3;
    let betain = one.div(beta);
    a = one;
    for _ in 1..=negep {
        a = a.mul(betain);
    }
    b = a;
    loop {
        let temp = one.sub(a);
        if temp.sub(one) != zero {
            break;
        }
        a = a.mul(beta);
        negep -= 1;
    }
    negep = -negep;
    let mut epsneg = a;
    if ibeta != 2 && irnd != 0 {
        a = a.mul(one.add(a)).div(two);
        let temp = one.sub(a);
        if temp.sub(one) != zero {
            epsneg = a;
        }
    }

    // determine machep, eps
    let mut machep = -it - 3;
    a = b;
    loop {
        let temp = one.add(a);
        if temp.sub(one) != zero {
            break;
        }
        a = a.mul(beta);
        machep += 1;
    }
    let mut eps = a;
    if ibeta != 2 && irnd != 0 {
        a = a.mul(one.add(a)).div(two);
        let temp = one.add(a);
        if temp.sub(one) != zero {
            eps = a;
        }
    }

    // determine ngrd
    let mut ngrd = 0;
    let temp = one.add(eps);
    if irnd == 0 && temp.mul(one).sub(one) != zero {
        ngrd = 1;
    }

    // determine iexp, minexp, xmin: the largest i and k = 2**i such that
    // (1/beta) ** (2**(i)) does not underflow; an underflow exits the loop
    let mut i = 0;
    let mut k = 1;
    let mut z = betain;
    let t = one.add(eps);
    let mut nxres = 0;
    let mut y;
    loop {
        y = z;
        z = y.mul(y);
        a = z.mul(one);
        let temp = z.mul(t);
        if a.add(a) == zero || z.abs() >= y {
            break;
        }
        let temp1 = temp.mul(betain);
        if temp1.mul(beta) == z {
            break;
        }
        i += 1;
        k += k;
    }
    // (ibeta is 2: the decimal machines' segment does not apply)
    let mut iexp = i + 1;
    let mut mx = k + k;
    let mut xmin;
    // loop to determine minexp, xmin; an underflow exits it
    loop {
        xmin = y;
        y = y.mul(betain);
        a = y.mul(one);
        let temp = y.mul(t);
        if a.add(a) == zero || y.abs() >= xmin {
            break;
        }
        k += 1;
        let temp1 = temp.mul(betain);
        if temp1.mul(beta) == y {
            nxres = 3;
            xmin = y;
            break;
        }
    }
    let minexp = -k;

    // determine maxexp, xmax
    if mx <= k + k - 3 && ibeta != 10 {
        mx += mx;
        iexp += 1;
    }
    let mut maxexp = mx + minexp;
    // adjust irnd to reflect partial underflow
    irnd += nxres;
    // adjust for ieee-style machines
    if irnd == 2 || irnd == 5 {
        maxexp -= 2;
    }
    // adjust for non-ieee machines with partial underflow
    if irnd == 3 || irnd == 4 {
        maxexp -= it;
    }
    // adjust for machines with implicit leading bit in binary significand,
    // and machines with radix point at extreme right of significand
    let i = maxexp + minexp;
    if ibeta == 2 && i == 0 {
        maxexp -= 1;
    }
    if i > 20 {
        maxexp -= 1;
    }
    if a != y {
        maxexp -= 2;
    }
    let mut xmax = one.sub(epsneg);
    if xmax.mul(one) != xmax {
        xmax = one.sub(beta.mul(epsneg));
    }
    xmax = xmax.div(beta.mul(beta).mul(beta).mul(xmin));
    let i = maxexp + minexp + 3;
    for _ in 1..=i.max(0) {
        xmax = xmax.add(xmax);
    }
    Machar { ibeta, it, irnd, ngrd, machep, negep, iexp, minexp, maxexp, eps, epsneg, xmin, xmax }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IEEE binary64: what GNU R 4.6.2 prints for `.Machine`'s double
    /// entries on x86-64.
    #[test]
    fn double() {
        let m = machar::<f64>();
        assert_eq!((m.ibeta, m.it, m.irnd, m.ngrd), (2, 53, 5, 0));
        assert_eq!((m.machep, m.negep, m.iexp, m.minexp, m.maxexp), (-52, -53, 11, -1022, 1024));
        assert_eq!((m.eps, m.epsneg), (f64::EPSILON, f64::EPSILON / 2.0));
        assert_eq!((m.xmin, m.xmax), (f64::MIN_POSITIVE, f64::MAX));
    }

    /// x87 extended: `.Machine`'s long double entries on x86-64.
    #[test]
    fn long_double() {
        let m = machar::<LDouble>();
        assert_eq!((m.ibeta, m.it, m.irnd, m.ngrd), (2, 64, 5, 0));
        assert_eq!((m.machep, m.negep, m.iexp, m.minexp, m.maxexp), (-63, -64, 15, -16382, 16384));
        assert_eq!((m.eps.to_f64(), m.epsneg.to_f64()), (2f64.powi(-63), 2f64.powi(-64)));
    }
}
