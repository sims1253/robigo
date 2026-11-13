//! Numbers: the common field of a vector (`formatReal`, `formatInteger`,
//! `formatLogical`, from format.c) and the encoding of one element in it
//! (`encodeReal0`, `EncodeRealDrop0`, `EncodeInteger`, `EncodeLogical`,
//! from printutils.c).
//!
//! `scientific()` scales by powers of ten in x87 `long double` and rounds
//! with `nearbyintl`, as GNU R does on x86_64 (`robigo_numerics::ldouble`).

use robigo_numerics::ldouble::LDouble;

use crate::{NA_INTEGER, PrintParams, cfmt, is_na, string};

/// `formatReal`'s result: `w` the field width, `d` the digits after the
/// decimal point (`%w.df`) or of the mantissa (`%w.de`), and `e` non-zero
/// for scientific format (2 when an exponent needs three digits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RealFormat {
    pub w: i32,
    pub d: i32,
    pub e: i32,
}

/// `scientific()`'s result for a finite `x`: `|x| = alpha * 10^kpower`
/// with `1 <= alpha < 10`, shown with `nsig` significant digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sci {
    /// `x < 0`.
    pub neg: bool,
    /// The decimal exponent after rounding to `digits` significant digits.
    pub kpower: i32,
    /// The significant digits needed, at most `digits`.
    pub nsig: i32,
    /// Rounding to `digits` makes the scientific form one digit wider than
    /// the fixed one would be (9996 at 3 digits is 1e+04 but prints 9996).
    pub roundingwidens: bool,
}

/// The largest power of ten in [`TBL`].
const KP_MAX: i32 = 27;

/// GNU R's `tbl[]`: declared `long double` but initialized from `double`
/// literals, so 1e23 to 1e27 are the nearest doubles, not exact powers.
const TBL: [f64; 28] = [
    1e00, 1e01, 1e02, 1e03, 1e04, 1e05, 1e06, 1e07, 1e08, 1e09, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18,
    1e19, 1e20, 1e21, 1e22, 1e23, 1e24, 1e25, 1e26, 1e27,
];

#[inline]
fn tbl(i: i32) -> LDouble {
    LDouble::from_f64(TBL[i as usize])
}

/// `DBL_DIG`.
const DBL_DIG: i32 = 15;

/// The decimal exponent and significant digits of a finite `x` printed
/// with `digits` (1 to 22) significant digits.
// R: format.c:scientific
pub fn scientific(x: f64, digits: i32) -> Sci {
    assert!((1..=22).contains(&digits), "R_print.digits out of range: {digits}");
    if x == 0.0 {
        return Sci { neg: false, kpower: 0, nsig: 1, roundingwidens: false };
    }
    let neg = x < 0.0;
    let r = x.abs();
    if digits >= DBL_DIG + 1 {
        let (kpower, nsig) = format_via_sprintf(r, digits);
        return Sci { neg, kpower, nsig, roundingwidens: false };
    }
    // r = |x|; 10^(kp + digits - 1) <= r
    let mut kp = r.log10().floor() as i32 - digits + 1;
    let mut r_prec = LDouble::from_f64(r);
    // use exact scaling factor in long double precision, if possible
    if kp.abs() <= KP_MAX {
        if kp > 0 {
            r_prec /= tbl(kp);
        } else if kp < 0 {
            r_prec *= tbl(-kp);
        }
    } else {
        r_prec /= LDouble::from_f64(10.0).powl(LDouble::from_i64(kp as i64));
    }
    if r_prec < tbl(digits - 1) {
        r_prec *= 10.0;
        kp -= 1;
    }
    // round alpha to integer, 10^(digits-1) <= alpha <= 10^digits;
    // accuracy limited by double rounding problem, alpha already rounded
    // to 64 bits
    let mut alpha = r_prec.rint().to_f64();
    let mut nsig = digits;
    for _ in 1..=digits {
        alpha /= 10.0;
        if alpha == alpha.floor() {
            nsig -= 1;
        } else {
            break;
        }
    }
    if nsig == 0 {
        nsig = 1;
        kp += 1;
    }
    let kpower = kp + digits - 1;
    // Scientific format may do more rounding than fixed format, e.g. 9996
    // with 3 digits is 1e+04 in scientific, but 9996 in fixed. This
    // happens when the true value r is less than 10^(kpower+1) and would
    // not round up to it in fixed format. Here rgt is the decimal place
    // that will be cut off by rounding.
    let rgt = (digits - kpower).clamp(0, KP_MAX);
    let fuzz = 0.5 / TBL[rgt as usize];
    // kpower can be bigger than the table.
    let roundingwidens = kpower > 0 && kpower <= KP_MAX && LDouble::from_f64(r) < tbl(kpower) - fuzz;
    Sci { neg, kpower, nsig, roundingwidens }
}

/// `scientific()` for 16 or more digits: read the exponent and the
/// trailing zeros off `%#.*e`.
// R: format.c:format_via_sprintf
fn format_via_sprintf(r: f64, d: i32) -> (i32, i32) {
    let mut buf = Vec::with_capacity(32);
    cfmt::exp(&mut buf, 0, (d - 1) as usize, r);
    // "D.DDD...e+XX": the exponent starts at d + 2.
    let kpower = std::str::from_utf8(&buf[d as usize + 2..]).ok().and_then(|s| s.parse().ok()).expect("an exponent");
    let mut i = d as usize;
    while i >= 2 && buf[i] == b'0' {
        i -= 1;
    }
    (kpower, i as i32)
}

/// The common format of the doubles in `x`: `nsmall` is the minimum number
/// of decimals in fixed format (0 except from `format()`).
// R: format.c:formatReal
pub fn format_real(x: &[f64], nsmall: i32, pp: &PrintParams) -> RealFormat {
    let (mut naflag, mut nanflag, mut posinf, mut neginf) = (false, false, false, false);
    let mut neg = 0;
    let mut mnl = i32::MAX;
    let (mut mxl, mut rgt, mut mxsl, mut mxns) = (i32::MIN, i32::MIN, i32::MIN, i32::MIN);
    for &xi in x {
        if !xi.is_finite() {
            if is_na(xi) {
                naflag = true;
            } else if xi.is_nan() {
                nanflag = true;
            } else if xi > 0.0 {
                posinf = true;
            } else {
                neginf = true;
            }
        } else {
            let sc = scientific(xi, pp.digits);
            let mut left = sc.kpower + 1;
            if sc.roundingwidens {
                left -= 1;
            }
            let sleft = sc.neg as i32 + if left <= 0 { 1 } else { left }; // >= 1
            let right = sc.nsig - left; // #{digits} right of '.' ( > 0 often)
            if sc.neg {
                neg = 1; // if any < 0, need extra space for sign
            }
            // Infinite precision "F" Format:
            rgt = rgt.max(right); // max digits to right of .
            mxl = mxl.max(left); // max digits to left of .
            mnl = mnl.min(left); // min digits to left of .
            mxsl = mxsl.max(sleft); // max left including sign(s)
            mxns = mxns.max(sc.nsig); // max sig digits
        }
    }
    // F Format: use "F" format WHENEVER we use not more space than 'E'
    // and still satisfy 'R_print.digits' {but as if nsmall==0 !}
    //
    // E Format has the form [S]X[.XXX]E+XX[X]
    //
    // This is indicated by setting *e to non-zero (usually 1). If the
    // additional exponent digit is required *e is set to 2.
    //
    // (C's `if (R_print.digits == 0) rgt = 0;` cannot apply: digits >= 1.)
    if mxl < 0 {
        mxsl = 1 + neg; // we use %#w.dg, so have leading zero
    }
    // use nsmall only *after* comparing "F" vs "E":
    if rgt < 0 {
        rgt = 0;
    }
    let mut wf = mxsl + rgt + (rgt != 0) as i32; // width for F format
    let mut f = RealFormat { w: 0, d: 0, e: if mxl > 100 || mnl <= -99 { 2 } else { 1 } };
    if mxns != i32::MIN {
        f.d = mxns - 1;
        f.w = neg + (f.d > 0) as i32 + f.d + 4 + f.e; // width for E format
        if wf <= f.w + pp.scipen {
            // Fixpoint if it needs less space
            f.e = 0;
            if nsmall > rgt {
                rgt = nsmall;
                wf = mxsl + rgt + (rgt != 0) as i32;
            }
            f.d = rgt;
            f.w = wf;
        } // else: "E" Exponential format -- all done above
    } else {
        // when all x[i] are non-finite
        f = RealFormat::default();
    }
    if naflag && f.w < pp.na_width {
        f.w = pp.na_width;
    }
    if nanflag && f.w < 3 {
        f.w = 3;
    }
    if posinf && f.w < 3 {
        f.w = 3;
    }
    if neginf && f.w < 4 {
        f.w = 4;
    }
    f
}

/// `IndexWidth(n)`: the decimal digits of `n` (1 for 0).
// R: printutils.c:IndexWidth
#[inline]
pub fn index_width(n: i64) -> i32 {
    ((n as f64 + 0.5).log10() + 1.0) as i32
}

/// The field width of the integers in `x`.
// R: format.c:formatInteger
pub fn format_integer(x: &[i32], pp: &PrintParams) -> i32 {
    let (mut xmin, mut xmax, mut naflag) = (i32::MAX, i32::MIN, false);
    for &v in x {
        if v == NA_INTEGER {
            naflag = true;
        } else {
            xmin = xmin.min(v);
            xmax = xmax.max(v);
        }
    }
    // R: format.c:FORMATINT_RETLOGIC
    let mut w = if naflag { pp.na_width } else { 1 };
    if xmin < 0 {
        w = w.max(index_width(-(xmin as i64)) + 1); // +1 for sign
    }
    if xmax > 0 {
        w = w.max(index_width(xmax as i64));
    }
    w
}

/// The field width of the logicals in `x` (`NA_INTEGER` is `NA`).
// R: format.c:formatLogical
pub fn format_logical(x: &[i32], pp: &PrintParams) -> i32 {
    let mut w = 1;
    for &v in x {
        if v == NA_INTEGER {
            if w < pp.na_width {
                w = pp.na_width;
            }
        } else if v != 0 && w < 4 {
            w = 4;
        } else if v == 0 && w < 5 {
            w = 5;
            break; // this is the widest it can be, so stop
        }
    }
    w
}

/// printutils.c's static buffers hold `NB` bytes: field widths are capped
/// at `NB - 1` and the text truncated to it.
const NB: usize = 1000;

#[inline]
fn cap(w: i32) -> i32 {
    w.min(NB as i32 - 1)
}

/// `snprintf(buff, NB, ...)`: keep at most `NB - 1` bytes of what was
/// appended after `start`.
#[inline]
fn truncate_nb(out: &mut Vec<u8>, start: usize) {
    out.truncate(start + (out.len() - start).min(NB - 1));
}

/// `x` in `fmt` (from [`format_real`]), right-justified in `fmt.w`, with
/// the decimal mark `dec`. `EncodeReal0` warns when `dec` is not one
/// character wide: see [`decimal_mark_warning`].
// R: printutils.c:EncodeReal0
pub fn encode_real0(out: &mut Vec<u8>, x: f64, fmt: RealFormat, dec: &[u8], pp: &PrintParams) {
    let start = out.len();
    encode_real_body(out, x, fmt, pp);
    replace_dec(out, start, dec);
}

/// [`encode_real0`] with the zeros after the decimal point dropped (before
/// the decimal mark is replaced): `as.character()` of a double.
// R: printutils.c:EncodeRealDrop0
pub fn encode_real_drop0(out: &mut Vec<u8>, x: f64, fmt: RealFormat, dec: &[u8], pp: &PrintParams) {
    let start = out.len();
    encode_real_body(out, x, fmt, pp);
    // Drop trailing zeroes
    if let Some(dot) = out[start..].iter().position(|&c| c == b'.') {
        let dot = start + dot;
        let mut p = dot + 1;
        let mut replace = dot;
        while p < out.len() && out[p].is_ascii_digit() {
            p += 1;
            if out[p - 1] != b'0' {
                replace = p;
            }
        }
        if replace != p {
            out.drain(replace..p);
        }
    }
    replace_dec(out, start, dec);
}

/// What `EncodeReal0`, `EncodeRealDrop0` and `EncodeReal2` share: `buff`.
fn encode_real_body(out: &mut Vec<u8>, x: f64, fmt: RealFormat, pp: &PrintParams) {
    let start = out.len();
    let w = cap(fmt.w);
    // IEEE allows signed zeros (yuck!)
    let x = if x == 0.0 { 0.0 } else { x };
    if !x.is_finite() {
        let s: &[u8] = if is_na(x) {
            pp.na_string.bytes()
        } else if x.is_nan() {
            b"NaN"
        } else if x > 0.0 {
            b"Inf"
        } else {
            b"-Inf"
        };
        cfmt::pad(out, w, s);
    } else if fmt.e != 0 {
        cfmt::exp(out, w, fmt.d as usize, x);
    } else {
        cfmt::fixed(out, w, fmt.d as usize, x);
    }
    truncate_nb(out, start);
}

/// `if(strcmp(dec, "."))`: replace every `.` after `start` by `dec`.
fn replace_dec(out: &mut Vec<u8>, start: usize, dec: &[u8]) {
    if dec == b"." {
        return;
    }
    let Some(first) = out[start..].iter().position(|&c| c == b'.') else { return };
    let tail = out.split_off(start + first);
    for &c in &tail {
        if c == b'.' {
            out.extend_from_slice(dec);
        } else {
            out.push(c);
        }
    }
}

/// The warning `EncodeReal0` and `EncodeRealDrop0` give on every call
/// with a decimal mark `dec` other than `.` that is not one character
/// wide (`Rstrwid` in the native encoding).
// R: printutils.c:EncodeReal0
pub fn decimal_mark_warning(dec: &[u8], pp: &PrintParams) -> Option<&'static str> {
    if dec == b"." {
        return None;
    }
    // 3·14 must work
    match string::rstrwid(dec, crate::Enc::Native, 0, pp.locale) {
        1 => None,
        n if n > 1 => Some("the decimal mark is more than one character wide; this will become an error"),
        _ => Some("the decimal mark is less than one character wide; this will become an error"),
    }
}

/// `x` (`NA_INTEGER` is `NA`) right-justified in `w`.
// R: printutils.c:EncodeInteger
pub fn encode_integer(out: &mut Vec<u8>, x: i32, w: i32, pp: &PrintParams) {
    let start = out.len();
    if x == NA_INTEGER {
        cfmt::pad(out, cap(w), pp.na_string.bytes());
    } else {
        cfmt::int(out, cap(w), x);
    }
    truncate_nb(out, start);
}

/// `x` (`NA_INTEGER` is `NA`) as `TRUE`/`FALSE`, right-justified in `w`.
// R: printutils.c:EncodeLogical
pub fn encode_logical(out: &mut Vec<u8>, x: i32, w: i32, pp: &PrintParams) {
    // fast path when 'w' fits exactly (R 4.6: the NA string is then not
    // padded even when its bytes are fewer than na_width)
    if x == NA_INTEGER {
        if w == pp.na_width {
            out.extend_from_slice(pp.na_string.bytes());
            return;
        }
    }
    let s: &[u8] = if x == NA_INTEGER {
        pp.na_string.bytes()
    } else if x != 0 {
        b"TRUE"
    } else {
        b"FALSE"
    };
    let start = out.len();
    cfmt::pad(out, cap(w), s);
    truncate_nb(out, start);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Str;

    fn enc(x: f64, digits: i32) -> String {
        let pp = PrintParams { digits, ..PrintParams::DEFAULT };
        let f = format_real(&[x], 0, &pp);
        let mut v = Vec::new();
        encode_real0(&mut v, x, f, b".", &pp);
        String::from_utf8(v).unwrap()
    }

    // Values from GNU R 4.6.2 (print()); tests/oracle.rs has the rest.
    #[test]
    fn basics() {
        assert_eq!(enc(1.0 / 3.0, 7), "0.3333333");
        assert_eq!(enc(1e5, 7), "1e+05");
        assert_eq!(enc(123456.0, 7), "123456");
        assert_eq!(enc(-0.0, 7), "0");
        assert_eq!(enc(9996.0, 3), "9996");
        assert_eq!(enc(0.1, 17), "0.10000000000000001");
        assert_eq!(enc(1e-300, 7), "1e-300");
        assert_eq!(enc(f64::INFINITY, 7), "Inf");
    }

    #[test]
    fn drop0() {
        let pp = PrintParams { digits: 15, ..PrintParams::DEFAULT };
        let f = RealFormat { w: 0, d: 4, e: 1 };
        let mut v = Vec::new();
        encode_real_drop0(&mut v, 1.5e-20, f, b",", &pp);
        assert_eq!(v, b"1,5e-20");
    }

    #[test]
    fn logical_fast_path() {
        // na.print = "\t": one byte, two wide; R 4.6 prints it unpadded.
        let pp = PrintParams { na_string: Str::ascii(b"\t"), na_width: 2, ..PrintParams::DEFAULT };
        let mut v = Vec::new();
        encode_logical(&mut v, NA_INTEGER, 2, &pp);
        assert_eq!(v, b"\t");
        v.clear();
        encode_integer(&mut v, NA_INTEGER, 2, &pp);
        assert_eq!(v, b" \t");
    }

    #[test]
    fn decimal_mark() {
        let pp = PrintParams::DEFAULT;
        assert_eq!(decimal_mark_warning(b",", &pp), None);
        assert_eq!(decimal_mark_warning("\u{b7}".as_bytes(), &pp), None);
        assert!(decimal_mark_warning(b"ab", &pp).unwrap().contains("more than"));
        assert!(decimal_mark_warning(b"", &pp).unwrap().contains("less than"));
    }
}
