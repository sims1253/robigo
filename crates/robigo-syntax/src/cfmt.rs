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
    match fmt_f_exact(x, prec) {
        Some(s) => s,
        None => format!("{:.*}", prec, x),
    }
}

/// `%.{prec}f` by integer arithmetic: `|x| = m * 2^e`, so `|x| * 10^prec`
/// rounded half to even (as glibc rounds the exact binary value) is
/// `m * 10^prec / 2^-e` with the remainder deciding. For `|x| < 1e15` and
/// `prec <= 17` everything fits in 128 bits; `None` otherwise.
fn fmt_f_exact(x: f64, prec: usize) -> Option<String> {
    let a = x.abs();
    if !a.is_finite() || a >= 1e15 || prec > 17 {
        return None;
    }
    let bits = a.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    // a = m * 2^e, with e < 0 since a < 2^50.
    let (m, e) = if exp == 0 { (frac, -1074) } else { (frac | (1u64 << 52), exp - 1075) };
    let p10 = 10u128.pow(prec as u32);
    let num = m as u128 * p10;
    let sh = (-e) as u32;
    let q = if sh >= 128 {
        0
    } else {
        let q = num >> sh;
        let r = num & ((1u128 << sh) - 1);
        let half = 1u128 << (sh - 1);
        if r > half || (r == half && q & 1 == 1) { q + 1 } else { q }
    };
    let mut s = String::with_capacity(24 + prec);
    if x.is_sign_negative() {
        s.push('-');
    }
    // (Both parts fit in 64 bits: q < 1e32 and 10^prec <= 1e17.)
    let (int, frac) = match u64::try_from(q) {
        Ok(q) => (q / p10 as u64, q % p10 as u64),
        Err(_) => ((q / p10) as u64, (q % p10) as u64),
    };
    push_u64(&mut s, int, 1);
    if prec > 0 {
        s.push('.');
        push_u64(&mut s, frac, prec);
    }
    Some(s)
}

/// Decimal digits of `v`, zero-padded to at least `width`.
fn push_u64(s: &mut String, mut v: u64, width: usize) {
    let mut buf = [b'0'; 24];
    let mut i = buf.len();
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let start = i.min(buf.len().saturating_sub(width));
    for &b in &buf[start..] {
        s.push(b as char);
    }
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
