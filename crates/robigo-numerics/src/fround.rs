//! `round(x, digits)` and `signif(x, digits)`: fround.c, fprec.c.

use super::*;

/// R: nmath/fround.c:fround (R >= 4.0.0 algorithm)
pub fn fround(x: f64, digits: f64) -> f64 {
    const MAX_DIGITS: f64 = (DBL_MAX_10_EXP + DBL_DIG) as f64;
    const MAX10E: i32 = DBL_MAX_10_EXP;

    if x.is_nan() || digits.is_nan() {
        return x + digits;
    }
    if !x.is_finite() {
        return x;
    }
    if digits > MAX_DIGITS || x == 0.0 {
        return x;
    } else if digits < -(MAX10E as f64) {
        return 0.0;
    } else if digits == 0.0 {
        return nearbyint(x);
    }

    let dig = (digits + 0.5).floor() as i32;
    let mut x = x;
    let mut sgn = 1.0;
    if x < 0.0 {
        sgn = -1.0;
        x = -x;
    }
    let l10x = M_LOG10_2 * (0.5 + logb(x));
    if l10x + dig as f64 > DBL_DIG as f64 {
        sgn * x
    } else {
        let (pow10, x10, i10, xd, xu);
        if dig <= MAX10E {
            pow10 = r_pow_di(10.0, dig);
            x10 = x * pow10;
            i10 = x10.floor();
            xd = i10 / pow10;
            xu = x10.ceil() / pow10;
        } else {
            let e10 = dig - MAX10E;
            let p10 = r_pow_di(10.0, e10);
            pow10 = r_pow_di(10.0, MAX10E);
            x10 = (x * pow10) * p10;
            i10 = x10.floor();
            xd = i10 / pow10 / p10;
            xu = x10.ceil() / pow10 / p10;
        }
        let du = xu - x;
        let dd = x - xd;
        sgn * (if du < dd || (fmod(i10, 2.0) == 1.0 && du == dd) { xu } else { xd })
    }
}

/// R: nmath/fprec.c:fprec
pub fn fprec(x: f64, digits: f64) -> f64 {
    const MAX_DIGITS: i32 = 22;
    const MAX10E: i32 = DBL_MAX_10_EXP;

    if x.is_nan() || digits.is_nan() {
        return x + digits;
    }
    if !x.is_finite() {
        return x;
    }
    let mut digits = digits;
    if !digits.is_finite() {
        if digits > 0.0 {
            return x;
        } else {
            digits = 1.0;
        }
    }
    if x == 0.0 {
        return x;
    }
    let mut dig = c_round(digits) as i32;
    if dig > MAX_DIGITS {
        return x;
    } else if dig < 1 {
        dig = 1;
    }
    let mut x = x;
    let mut sgn = 1.0;
    if x < 0.0 {
        sgn = -sgn;
        x = -x;
    }
    let l10 = x.log10();
    let mut e10 = (dig as f64 - 1.0 - l10.floor()) as i32;
    if l10.abs() < (MAX10E - 2) as f64 {
        let mut p10 = 1.0;
        if e10 > MAX10E {
            p10 = r_pow_di(10.0, e10 - MAX10E);
            e10 = MAX10E;
        }
        if e10 > 0 {
            let pow10 = r_pow_di(10.0, e10);
            sgn * (nearbyint((x * pow10) * p10) / pow10) / p10
        } else {
            let pow10 = r_pow_di(10.0, -e10);
            sgn * (nearbyint(x / pow10) * pow10)
        }
    } else {
        let do_round = MAX10E as f64 - l10 >= r_pow_di(10.0, -dig);
        let e2 = dig + (if e10 > 0 { 1 } else { -1 }) * MAX_DIGITS;
        let p10 = r_pow_di(10.0, e2);
        x *= p10;
        let pp10 = r_pow_di(10.0, e10 - e2);
        x *= pp10;
        if do_round {
            x += 0.5;
        }
        x = x.floor() / p10;
        sgn * x / pp10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding() {
        assert_eq!(fround(0.15, 1.0), 0.1);
        assert_eq!(fround(2.5, 0.0), 2.0);
        assert_eq!(fround(-2.5, 0.0), -2.0);
        assert_eq!(fround(0.125, 2.0), 0.12);
        assert_eq!(fround(2.675, 2.0), 2.67);
        assert_eq!(fround(1234.5678, -2.0), 1200.0);
        assert_eq!(fprec(123456.0, 2.0), 120000.0);
        assert_eq!(fprec(0.000123456, 3.0), 0.000123);
        assert_eq!(fprec(-1.5e300, 1.0), -1.0000000000000006e+300);
    }
}

/// Truncation toward zero (Rmath's `Rf_ftrunc`; C code calls it).
// R: ftrunc.c:ftrunc
pub fn ftrunc(x: f64) -> f64 {
    x.trunc()
}
