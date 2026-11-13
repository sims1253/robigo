//! The subset of C's `snprintf` the formatters use: `%*s`, `%*d`,
//! `%*.*e` and `%*.*f` on finite doubles, as glibc prints them (correctly
//! rounded from the exact binary value, ties to even, exponents with at
//! least two digits).

use std::io::Write;

/// Append `n` spaces.
#[inline]
pub(crate) fn spaces(out: &mut Vec<u8>, n: usize) {
    out.resize(out.len() + n, b' ');
}

/// `%*s` (and the field width of every conversion): `body` right-justified
/// in `w` bytes; a negative `w` left-justifies in `-w` (C's `-` flag).
#[inline]
pub(crate) fn pad(out: &mut Vec<u8>, w: i32, body: &[u8]) {
    let n = w.unsigned_abs() as usize;
    let fill = n.saturating_sub(body.len());
    if w >= 0 {
        spaces(out, fill);
        out.extend_from_slice(body);
    } else {
        out.extend_from_slice(body);
        spaces(out, fill);
    }
}

/// `%*d`.
pub(crate) fn int(out: &mut Vec<u8>, w: i32, x: i32) {
    let mut buf = [0u8; 12];
    let mut c = std::io::Cursor::new(&mut buf[..]);
    write!(c, "{x}").expect("an i32 fits in 12 bytes");
    let n = c.position() as usize;
    pad(out, w, &buf[..n]);
}

/// `%*.*e` of a finite `x` (the `#` flag changes nothing when `prec > 0`,
/// the only way R uses it).
pub(crate) fn exp(out: &mut Vec<u8>, w: i32, prec: usize, x: f64) {
    // Rust's `{:e}` is correctly rounded with ties to even, like glibc;
    // only the exponent is spelled differently ("1.5e-7", not "1.5e-07").
    let s = format!("{x:.prec$e}");
    let (mant, e) = s.split_once('e').expect("an exponent");
    let (sign, digits) = match e.strip_prefix('-') {
        Some(d) => (b'-', d),
        None => (b'+', e),
    };
    let mut body = Vec::with_capacity(mant.len() + 5);
    body.extend_from_slice(mant.as_bytes());
    body.push(b'e');
    body.push(sign);
    if digits.len() < 2 {
        body.push(b'0');
    }
    body.extend_from_slice(digits.as_bytes());
    pad(out, w, &body);
}

/// `%*.*f` of a finite `x`.
pub(crate) fn fixed(out: &mut Vec<u8>, w: i32, prec: usize, x: f64) {
    let s = format!("{x:.prec$}");
    pad(out, w, s.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: impl FnOnce(&mut Vec<u8>)) -> String {
        let mut v = Vec::new();
        f(&mut v);
        String::from_utf8(v).unwrap()
    }

    // Expected values: glibc's printf.
    #[test]
    fn like_glibc() {
        assert_eq!(run(|o| exp(o, 0, 3, 12345.678)), "1.235e+04");
        assert_eq!(run(|o| exp(o, 0, 0, 1e-300)), "1e-300");
        assert_eq!(run(|o| exp(o, 10, 1, -1.25)), "  -1.2e+00");
        assert_eq!(run(|o| exp(o, 0, 0, 25.0)), "2e+01");
        assert_eq!(run(|o| exp(o, 0, 2, 5e-324)), "4.94e-324");
        assert_eq!(run(|o| fixed(o, 0, 2, 0.125)), "0.12");
        assert_eq!(run(|o| fixed(o, 0, 0, 2.5)), "2");
        assert_eq!(run(|o| fixed(o, 0, 0, -0.25)), "-0");
        assert_eq!(run(|o| fixed(o, 0, 0, 1e23)), "99999999999999991611392");
        assert_eq!(run(|o| fixed(o, -6, 1, 1.0)), "1.0   ");
        assert_eq!(run(|o| int(o, 5, -12)), "  -12");
        assert_eq!(run(|o| int(o, 0, i32::MIN + 1)), "-2147483647");
        assert_eq!(run(|o| pad(o, 3, b"abcd")), "abcd");
    }
}
