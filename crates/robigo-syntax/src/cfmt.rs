//! C `printf`-compatible formatting of finite doubles (`%e`, `%f`, `%g`).
//!
//! GNU R formats numbers through the C library, so matching its output means
//! matching glibc: correctly rounded decimal conversion, ties to even, and
//! exponents with at least two digits.

/// `%.{prec}e`
pub fn fmt_e(x: f64, prec: usize, upper: bool) -> String {
    let s = format!("{:.*e}", prec, x);
    let (mant, exp) = s.split_once('e').expect("exponent");
    let exp: i32 = exp.parse().expect("exponent digits");
    let sign = if exp < 0 { '-' } else { '+' };
    let e = if upper { 'E' } else { 'e' };
    format!("{}{}{}{:02}", mant, e, sign, exp.abs())
}

/// `%.{prec}f`
pub fn fmt_f(x: f64, prec: usize) -> String {
    format!("{:.*}", prec, x)
}

/// Decimal exponent of `x` after rounding to `prec` significant digits.
pub fn exponent_after_rounding(x: f64, sig: usize) -> i32 {
    if x == 0.0 {
        return 0;
    }
    let s = format!("{:.*e}", sig.max(1) - 1, x);
    s.split_once('e').unwrap().1.parse().unwrap()
}

/// `%.{prec}g`, with `alt` for the `#` flag (keep trailing zeros).
pub fn fmt_g(x: f64, prec: usize, alt: bool, upper: bool) -> String {
    let p = if prec == 0 { 1 } else { prec };
    let x_exp = exponent_after_rounding(x, p);
    let mut s = if x_exp < -4 || x_exp >= p as i32 {
        fmt_e(x, p - 1, upper)
    } else {
        fmt_f(x, (p as i32 - 1 - x_exp) as usize)
    };
    if !alt {
        s = strip_trailing_zeros(&s);
    } else if !s.contains('.') {
        // `#` forces a decimal point.
        match s.find(['e', 'E']) {
            Some(i) => s.insert(i, '.'),
            None => s.push('.'),
        }
    }
    s
}

fn strip_trailing_zeros(s: &str) -> String {
    let (mant, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    if !mant.contains('.') {
        return s.to_string();
    }
    let mant = mant.trim_end_matches('0').trim_end_matches('.');
    format!("{}{}", mant, exp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g17() {
        assert_eq!(fmt_g(0.1, 17, false, false), "0.10000000000000001");
        assert_eq!(fmt_g(1e300, 17, false, false), "1.0000000000000001e+300");
        assert_eq!(fmt_g(100.0, 17, false, false), "100");
        assert_eq!(fmt_g(1e-5, 17, false, false), "1.0000000000000001e-05");
        assert_eq!(fmt_g(123456789012345678.0, 17, false, false), "1.2345678901234568e+17");
        assert_eq!(fmt_g(0.0, 17, false, false), "0");
        assert_eq!(fmt_g(-0.0, 17, false, false), "-0");
    }

    #[test]
    fn e_and_f() {
        assert_eq!(fmt_e(12345.678, 3, false), "1.235e+04");
        assert_eq!(fmt_e(1e-300, 0, false), "1e-300");
        assert_eq!(fmt_f(0.125, 2), "0.12");
        assert_eq!(fmt_f(2.5, 0), "2");
    }
}
