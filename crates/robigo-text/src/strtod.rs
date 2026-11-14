//! util.c's number parser, `R_strtod5` and its short forms: what
//! `as.numeric("1e3")`, `as.integer(" 12 ")` and the coercions of
//! character vectors read (coerce.c's `String` conversions), with digits
//! accumulated and scaled in x87 long double as GNU R does, so results
//! (double rounding included) are GNU R's.

use crate::NA_REAL;

/// GNU R's `isBlankString`: only `isspace` characters (in the C locale:
/// space, `\t`, `\n`, `\v`, `\f`, `\r`).
// R: util.c:isBlankString (the byte loop; robigo-rt runs in a UTF-8
// locale where the C library's iswspace agrees on ASCII)
pub fn is_blank(s: &[u8]) -> bool {
    s.iter().all(|&c| matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
}

/// `R_strtod(str, &endp)`: [`r_strtod5`] with `.` as the decimal point, no
/// "NA" and no exactness check.
// R: util.c:R_strtod
pub fn r_strtod(s: &[u8]) -> (f64, usize) {
    r_strtod5(s, b'.', false, 0)
}

/// `R_strtod5(str, &endp, dec, NA, exact)`: digits accumulated and scaled in long double, so
/// results (including double rounding) are GNU R's. Returns the value and
/// the number of bytes consumed (0 if nothing was parsed). `dec` is the
/// decimal point, `na` accepts "NA", `exact` is type.convert's
/// `numerals` handling (0: allow loss, 1: NA on loss, NA_LOGICAL: warn;
/// the warning is the caller's, not given here).
// R: util.c:R_strtod5
pub fn r_strtod5(s: &[u8], dec: u8, na: bool, exact: i32) -> (f64, usize) {
    use robigo_numerics::ldouble::LDouble as LD;
    let at = |p: usize| s.get(p).copied().unwrap_or(0);
    let starts_ci = |p: usize, w: &[u8]| s.len() >= p + w.len() && s[p..p + w.len()].eq_ignore_ascii_case(w);
    let mut p = 0;
    while matches!(at(p), b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        p += 1;
    }
    if na && s[p..].starts_with(b"NA") {
        return (NA_REAL, p + 2);
    }
    let mut sign = 1.0;
    match at(p) {
        b'-' => {
            sign = -1.0;
            p += 1;
        }
        b'+' => p += 1,
        _ => {}
    }
    match at(p) {
        b'n' | b'N' if starts_ci(p, b"NaN") => return (sign * f64::NAN, p + 3),
        b'i' | b'I' => {
            if starts_ci(p, b"infinity") {
                return (sign * f64::INFINITY, p + 8);
            } else if starts_ci(p, b"Inf") {
                return (sign * f64::INFINITY, p + 3);
            }
        }
        _ => {}
    }
    let back_out = (NA_REAL, 0usize);
    let mut ans = LD::ZERO;
    let mut expn: i64 = 0;
    let max_exp_prefix: i64 = 9999;
    let exact_clause = |ans: LD| -> Option<bool> {
        // Some(true): back out with NA; Some(false): warn.
        if exact != 0 && ans > LD::from_f64(f64::from_bits(0x433f_ffff_ffff_ffff)) {
            Some(exact != i32::MIN)
        } else {
            None
        }
    };
    let mut warn_loss = false;
    if s.len() > p + 2 && at(p) == b'0' && (at(p + 1) == b'x' || at(p + 1) == b'X') {
        let mut exph: i64 = -1;
        p += 2;
        loop {
            let c = at(p);
            let d = match c {
                b'0'..=b'9' => (c - b'0') as f64,
                b'a'..=b'f' => (c - b'a' + 10) as f64,
                b'A'..=b'F' => (c - b'A' + 10) as f64,
                _ if c == dec && c != 0 => {
                    exph = 0;
                    p += 1;
                    continue;
                }
                _ => break,
            };
            ans = ans * 16.0 + d;
            if exph >= 0 {
                exph += 4;
            }
            p += 1;
        }
        match exact_clause(ans) {
            Some(true) => return back_out,
            Some(false) => warn_loss = true,
            None => {}
        }
        if at(p) == b'p' || at(p) == b'P' {
            p += 1;
            let mut expsign = 1;
            match at(p) {
                b'-' => {
                    expsign = -1;
                    p += 1;
                }
                b'+' => p += 1,
                _ => {}
            }
            let (mut n, mut ndig) = (0i64, 0);
            while at(p).is_ascii_digit() {
                n = if n < max_exp_prefix { n * 10 + (at(p) - b'0') as i64 } else { n };
                p += 1;
                ndig += 1;
            }
            if ndig == 0 {
                return back_out;
            }
            expn += expsign * n;
        }
        if ans != LD::from_f64(0.0) {
            let pow = |mut n: i64, base: f64| {
                let (mut fac, mut b) = (LD::from_f64(1.0), base);
                while n != 0 {
                    if n & 1 != 0 {
                        fac *= b;
                    }
                    n >>= 1;
                    b *= b;
                }
                fac
            };
            if exph > 0 {
                if expn - exph < -122 {
                    ans /= pow(exph, 2.0);
                } else {
                    expn -= exph;
                }
            }
            if expn < 0 {
                ans /= pow(-expn, 2.0);
            } else {
                ans *= pow(expn, 2.0);
            }
        }
        let _ = warn_loss;
        return (sign * ans.to_f64(), p);
    }
    let mut ndigits: i64 = 0;
    // `ans = 10 * ans + digit` in long double is exact while ans < 2^63:
    // accumulate in an integer until then.
    let mut acc: u64 = 0;
    let mut big = false;
    #[cold]
    #[inline(never)]
    fn big_digit(ans: &mut LD, acc: u64, big: &mut bool, d: u8) {
        if !*big {
            *ans = LD::from_i64(acc as i64);
            *big = true;
        }
        *ans = *ans * 10.0 + d as f64;
    }
    let mut digit = |ans: &mut LD, d: u8| {
        if !big && acc < (i64::MAX as u64 - 9) / 10 {
            acc = acc * 10 + d as u64;
        } else {
            big_digit(ans, acc, &mut big, d);
        }
    };
    while at(p).is_ascii_digit() {
        digit(&mut ans, at(p) - b'0');
        p += 1;
        ndigits += 1;
    }
    if at(p) == dec && dec != 0 {
        p += 1;
        while at(p).is_ascii_digit() {
            digit(&mut ans, at(p) - b'0');
            p += 1;
            ndigits += 1;
            expn -= 1;
        }
    }
    if ndigits == 0 {
        return back_out;
    }
    let loss = if big {
        exact_clause(ans)
    } else if exact != 0 && acc > 9_007_199_254_740_991 {
        Some(exact != i32::MIN)
    } else {
        None
    };
    match loss {
        Some(true) => return back_out,
        Some(false) => warn_loss = true,
        None => {}
    }
    let _ = warn_loss;
    if at(p) == b'e' || at(p) == b'E' {
        p += 1;
        let mut expsign = 1;
        match at(p) {
            b'-' => {
                expsign = -1;
                p += 1;
            }
            b'+' => p += 1,
            _ => {}
        }
        let (mut n, mut nd) = (0i64, 0);
        while at(p).is_ascii_digit() {
            n = if n < max_exp_prefix { n * 10 + (at(p) - b'0') as i64 } else { n };
            p += 1;
            nd += 1;
        }
        if nd == 0 {
            return back_out;
        }
        expn += expsign * n;
    }
    // R: util.c:R_strtod5 uses positive powers as the divisor. Through
    // 10^18 these integers fit i64 and are exact in x87 long double, so
    // constructing them directly gives the same fac as repeated squaring.
    const POW10: [i64; 19] = {
        let mut powers = [1; 19];
        let mut i = 1;
        while i < powers.len() {
            powers[i] = powers[i - 1] * 10;
            i += 1;
        }
        powers
    };
    if !big {
        if let Some(&power) = POW10.get(expn.unsigned_abs() as usize) {
            // Both integer operands are exact in long double. The result
            // is bounded by i64::MAX * 10^18, well below f64 overflow;
            // exponents in this range need no underflow rescaling.
            let value =
                if expn < 0 { LD::div_i64_to_f64(acc as i64, power) } else { LD::mul_i64_to_f64(acc as i64, power) };
            return (sign * value, p);
        }
        ans = LD::from_i64(acc as i64);
    }
    if expn + ndigits < -300 {
        for _ in 0..ndigits {
            ans /= 10.0;
        }
        expn += ndigits;
    }
    if let Some(&power) = POW10.get(expn.unsigned_abs() as usize) {
        if expn < 0 {
            ans /= LD::from_i64(power);
        } else if ans != LD::from_f64(0.0) {
            ans *= LD::from_i64(power);
        }
    } else {
        let (mut p10, mut fac) = (LD::from_f64(10.0), LD::from_f64(1.0));
        if expn < -307 {
            let mut n = -expn;
            while n != 0 {
                if n & 1 != 0 {
                    fac /= p10;
                }
                n >>= 1;
                p10 = p10 * p10;
            }
            ans *= fac;
        } else if expn < 0 {
            let mut n = -expn;
            while n != 0 {
                if n & 1 != 0 {
                    fac *= p10;
                }
                n >>= 1;
                p10 = p10 * p10;
            }
            ans /= fac;
        } else if ans != LD::from_f64(0.0) {
            let mut n = expn;
            while n != 0 {
                if n & 1 != 0 {
                    fac *= p10;
                }
                n >>= 1;
                p10 = p10 * p10;
            }
            ans *= fac;
        }
    }
    if ans > LD::from_f64(f64::MAX) {
        return (sign * f64::INFINITY, p);
    }
    (sign * ans.to_f64(), p)
}
