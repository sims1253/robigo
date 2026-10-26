//! Number and string formatting for printing. Port of GNU R's format.c and
//! printutils.c.
//!
//! `scientific()` scales by powers of ten in 80-bit `long double` and
//! rounds with `nearbyintl`, as GNU R does on x86_64 (see `nmath::ldouble`).

use robigo_syntax::cfmt::{fmt_e, fmt_f};

use crate::value::*;

#[derive(Clone, Copy, Debug)]
pub struct NumFormat {
    pub digits: usize,
}

/// R's `R_print` settings.
#[derive(Clone, Debug)]
pub struct PrintParams {
    pub digits: usize,
    pub scipen: i32,
    pub width: usize,
    pub gap: usize,
    pub quote: bool,
    pub right: bool,
    pub max: usize,
    pub na_string: String,
    pub na_string_noquote: String,
    pub na_width: usize,
    pub na_width_noquote: usize,
}

impl Default for PrintParams {
    fn default() -> Self {
        PrintParams {
            digits: 7,
            scipen: 0,
            width: 80,
            gap: 1,
            quote: true,
            right: false,
            max: 99999,
            na_string: "NA".into(),
            na_string_noquote: "<NA>".into(),
            na_width: 2,
            na_width_noquote: 4,
        }
    }
}

use crate::nmath::ldouble::LDouble;

/// The largest power of ten in `tbl`.
const KP_MAX: i32 = 27;

/// GNU R's `tbl[]`: declared `long double`, but initialized with `double`
/// literals, so 1e23 to 1e27 are the nearest doubles, not exact powers.
const TBL: [f64; 28] = [
    1e00, 1e01, 1e02, 1e03, 1e04, 1e05, 1e06, 1e07, 1e08, 1e09, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17,
    1e18, 1e19, 1e20, 1e21, 1e22, 1e23, 1e24, 1e25, 1e26, 1e27,
];

fn tbl(i: i32) -> LDouble {
    LDouble::from_f64(TBL[i as usize])
}

pub struct Sci {
    pub neg: bool,
    pub kpower: i32,
    pub nsig: i32,
    pub roundingwidens: bool,
}

/// For finite `x`, the decimal exponent and number of significant digits
/// (at most `digits`) needed to represent it.
// R: format.c:scientific
pub fn scientific(x: f64, digits: usize) -> Sci {
    if x == 0.0 {
        return Sci { neg: false, kpower: 0, nsig: 1, roundingwidens: false };
    }
    let neg = x < 0.0;
    let r = x.abs();
    let digits = digits.max(1) as i32;
    if digits >= 16 {
        // format_via_sprintf: "%#.*e" (Rust's conversion is correctly
        // rounded, as glibc's is).
        let s = format!("{:.*e}", digits as usize - 1, r);
        let (mant, exp) = s.split_once('e').unwrap();
        let kpower: i32 = exp.parse().unwrap();
        let mant = mant.as_bytes();
        // "d.ddd": digit i (1-based, after the point) is mant[i + 1].
        let mut nsig = digits;
        while nsig >= 2 && mant[nsig as usize] == b'0' {
            nsig -= 1;
        }
        return Sci { neg, kpower, nsig, roundingwidens: false };
    }
    // r = |x|; 10^(kp + digits - 1) <= r
    let mut kp = r.log10().floor() as i32 - digits + 1;
    let mut r_prec = LDouble::from_f64(r);
    // Use the exact scaling factor in long double precision, if possible.
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
    // Round alpha to an integer, 10^(digits-1) <= alpha <= 10^digits.
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
    // with 3 digits is 1e+04 in scientific, but 9996 in fixed.
    let rgt = (digits - kpower).clamp(0, KP_MAX);
    let fuzz = 0.5 / TBL[rgt as usize];
    let roundingwidens = kpower > 0 && kpower <= KP_MAX && LDouble::from_f64(r) < tbl(kpower) - fuzz;
    Sci { neg, kpower, nsig, roundingwidens }
}

/// `formatReal`: common (width, decimals, exponent-digits) for a vector.
pub fn format_real(x: &[f64], digits: usize, scipen: i32, nsmall: usize, na_width: usize) -> (usize, usize, usize) {
    let (mut naflag, mut nanflag, mut posinf, mut neginf) = (false, false, false, false);
    let mut neg = 0i32;
    let mut mnl = i32::MAX;
    let (mut mxl, mut rgt, mut mxsl, mut mxns) = (i32::MIN, i32::MIN, i32::MIN, i32::MIN);
    for &xi in x {
        if !xi.is_finite() {
            if is_na_real(xi) {
                naflag = true;
            } else if xi.is_nan() {
                nanflag = true;
            } else if xi > 0.0 {
                posinf = true;
            } else {
                neginf = true;
            }
        } else {
            let sc = scientific(xi, digits);
            let mut left = sc.kpower + 1;
            if sc.roundingwidens {
                left -= 1;
            }
            let sleft = sc.neg as i32 + if left <= 0 { 1 } else { left };
            let right = sc.nsig - left;
            if sc.neg {
                neg = 1;
            }
            rgt = rgt.max(right);
            mxl = mxl.max(left);
            mnl = mnl.min(left);
            mxsl = mxsl.max(sleft);
            mxns = mxns.max(sc.nsig);
        }
    }
    if digits == 0 {
        rgt = 0;
    }
    if mxl < 0 {
        mxsl = 1 + neg;
    }
    if rgt < 0 {
        rgt = 0;
    }
    let mut wf = mxsl + rgt + (rgt != 0) as i32;
    let mut e = if mxl > 100 || mnl <= -99 { 2 } else { 1 };
    let (mut w, mut d);
    if mxns != i32::MIN {
        d = mxns - 1;
        w = neg + (d > 0) as i32 + d + 4 + e;
        if wf <= w + scipen {
            e = 0;
            if nsmall as i32 > rgt {
                rgt = nsmall as i32;
                wf = mxsl + rgt + (rgt != 0) as i32;
            }
            d = rgt;
            w = wf;
        }
    } else {
        w = 0;
        d = 0;
        e = 0;
    }
    let mut w = w.max(0) as usize;
    if naflag {
        w = w.max(na_width);
    }
    if nanflag {
        w = w.max(3);
    }
    if posinf {
        w = w.max(3);
    }
    if neginf {
        w = w.max(4);
    }
    (w, d.max(0) as usize, e as usize)
}

fn pad_left(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n >= w {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(w - n), s)
    }
}

/// `EncodeReal0`.
pub fn encode_real0(x: f64, w: usize, d: usize, e: usize, na: &str) -> String {
    let x = if x == 0.0 { 0.0 } else { x };
    let s = if !x.is_finite() {
        if is_na_real(x) {
            na.to_string()
        } else if x.is_nan() {
            "NaN".into()
        } else if x > 0.0 {
            "Inf".into()
        } else {
            "-Inf".into()
        }
    } else if e > 0 {
        let mut s = fmt_e(x, d, false);
        if d > 0 && !s.contains('.') {
            // `%#.*e` keeps the decimal point; only matters for d == 0.
            s = s.replacen('e', ".e", 1);
        }
        // glibc pads the exponent to two digits; with e == 2 the exponent
        // naturally has three digits.
        s
    } else {
        fmt_f(x, d)
    };
    pad_left(&s, w)
}

/// `EncodeRealDrop0`: like [`encode_real0`] but dropping trailing zeros.
pub fn encode_real_drop0(x: f64, w: usize, d: usize, e: usize, na: &str) -> String {
    let mut s = encode_real0(x, w, d, e, na);
    drop_trailing_zeros_in(&mut s);
    s
}

pub fn drop_trailing_zeros(s: &str) -> String {
    let mut s = s.to_string();
    drop_trailing_zeros_in(&mut s);
    s
}

/// Drop the zeros at the end of the digits after the decimal point (and
/// the point if none remain).
// R: printutils.c:dropTrailing0
fn drop_trailing_zeros_in(s: &mut String) {
    let Some(dot) = s.find('.') else { return };
    let bytes = s.as_bytes();
    let mut end = dot + 1;
    let mut replace = dot;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        if bytes[end] != b'0' {
            replace = end + 1;
        }
        end += 1;
    }
    if replace != end {
        s.replace_range(replace..end, "");
    }
}

/// `as.character()` of a double: 15 significant digits, trailing zeros
/// dropped.
pub fn string_from_real(x: f64) -> RStr {
    if is_na_real(x) {
        return None;
    }
    let (w, d, e) = format_real(&[x], 15, 0, 0, 2);
    rstr(&encode_real_drop0(x, w, d, e, "NA"))
}

pub fn index_width(n: usize) -> usize {
    let mut n = n;
    let mut w = 1;
    while n >= 10 {
        n /= 10;
        w += 1;
    }
    w
}

pub fn format_integer(x: &[i32], na_width: usize) -> usize {
    let mut naflag = false;
    let (mut xmin, mut xmax) = (i32::MAX, i32::MIN);
    for &v in x {
        if v == NA_INTEGER {
            naflag = true;
        } else {
            xmin = xmin.min(v);
            xmax = xmax.max(v);
        }
    }
    let mut w = if naflag { na_width } else { 1 };
    if xmin < 0 {
        w = w.max(index_width(-(xmin as i64) as usize) + 1);
    }
    if xmax > 0 {
        w = w.max(index_width(xmax as usize));
    }
    w
}

pub fn encode_integer(x: i32, w: usize, na: &str) -> String {
    if x == NA_INTEGER {
        pad_left(na, w)
    } else {
        pad_left(&x.to_string(), w)
    }
}

pub fn format_logical(x: &[Lgl], na_width: usize) -> usize {
    let mut w = 1;
    for &v in x {
        if v == NA_LOGICAL {
            w = w.max(na_width);
        } else if v != 0 && w < 4 {
            w = 4;
        } else if v == 0 && w < 5 {
            w = 5;
            break;
        }
    }
    w
}

pub fn encode_logical(x: Lgl, w: usize, na: &str) -> String {
    let s = if x == NA_LOGICAL {
        na
    } else if x != 0 {
        "TRUE"
    } else {
        "FALSE"
    };
    pad_left(s, w)
}

/// Format info for complex: ((wr, dr, er), (wi, di, ei)).
pub fn format_complex(
    x: &[Complex],
    digits: usize,
    scipen: i32,
    nsmall: usize,
    na_width: usize,
) -> ((usize, usize, usize), (usize, usize, usize)) {
    let mut re = Vec::new();
    let mut im = Vec::new();
    let mut naflag = false;
    for z in x {
        if is_na_real(z.re) || is_na_real(z.im) {
            naflag = true;
        } else {
            re.push(z.re);
            im.push(z.im.abs());
        }
    }
    let r = format_real(&re, digits, scipen, nsmall, na_width);
    let i = format_real(&im, digits, scipen, nsmall, na_width);
    let mut r = r;
    if naflag && r.0 + i.0 + 2 < na_width {
        r.0 += na_width - (r.0 + i.0 + 2);
    }
    (r, i)
}

pub fn encode_complex(z: Complex, wr: (usize, usize, usize), wi: (usize, usize, usize), na: &str) -> String {
    if is_na_real(z.re) || is_na_real(z.im) {
        return pad_left(na, wr.0 + wi.0 + 2);
    }
    let re = if z.re == 0.0 { 0.0 } else { z.re };
    let mut imv = if z.im == 0.0 { 0.0 } else { z.im };
    let re_s = encode_real0(re, wr.0, wr.1, wr.2, na);
    let mut neg = imv < 0.0;
    if neg {
        imv = -imv;
    }
    let im_s = encode_real0(imv, wi.0, wi.1, wi.2, na);
    if im_s == "0" {
        neg = false;
    }
    format!("{}{}{}i", re_s, if neg { "-" } else { "+" }, im_s)
}

/// Display width of a string (East Asian wide characters count double).
pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

pub fn char_width(c: char) -> usize {
    let cp = c as u32;
    if cp == 0 {
        return 0;
    }
    // Combining marks.
    if (0x300..=0x36F).contains(&cp) || (0x200B..=0x200F).contains(&cp) {
        return 0;
    }
    if (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0xA4CF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE30..=0xFE4F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp)
        || (0x1F300..=0x1F64F).contains(&cp)
        || (0x1F900..=0x1F9FF).contains(&cp)
        || (0x20000..=0x3FFFD).contains(&cp)
    {
        return 2;
    }
    1
}

/// Escape a string for printing (R's `EncodeString` without padding).
pub fn escape_string(s: &str, quote: Option<char>) -> String {
    match quote {
        Some(q) => robigo_syntax::dump::encode_string(s, q),
        None => {
            let quoted = robigo_syntax::dump::encode_string(s, '"');
            // Strip the quotes and un-escape `\"`.
            let inner = &quoted[1..quoted.len() - 1];
            inner.replace("\\\"", "\"")
        }
    }
}

/// `Rstrlen`: display width of the escaped (and maybe quoted) string.
pub fn rstrlen(s: &str, quote: Option<char>) -> usize {
    str_width(&escape_string(s, quote))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Justify {
    Left,
    Right,
    Centre,
    None,
}

/// `EncodeString(s, w, quote, justify)`.
pub fn encode_string(s: &RStr, w: usize, quote: Option<char>, justify: Justify, pp: &PrintParams) -> String {
    let body = match s {
        None => {
            if quote.is_some() {
                pp.na_string.clone()
            } else {
                pp.na_string_noquote.clone()
            }
        }
        Some(s) => escape_string(s, quote),
    };
    let bw = str_width(&body);
    if bw >= w || justify == Justify::None {
        return body;
    }
    let pad = w - bw;
    match justify {
        Justify::Left => format!("{}{}", body, " ".repeat(pad)),
        Justify::Right => format!("{}{}", " ".repeat(pad), body),
        Justify::Centre => {
            let l = pad / 2;
            format!("{}{}{}", " ".repeat(l), body, " ".repeat(pad - l))
        }
        Justify::None => body,
    }
}

pub fn format_string(x: &[RStr], quote: Option<char>, pp: &PrintParams) -> usize {
    let mut w = 0;
    for s in x {
        let l = match s {
            None => {
                if quote.is_some() {
                    pp.na_width
                } else {
                    pp.na_width_noquote
                }
            }
            Some(s) => rstrlen(s, quote),
        };
        w = w.max(l);
    }
    w
}

/// Element encoding used by deparse (digits = 15, no padding).
pub fn encode_element_for_deparse(v: &Value, i: usize) -> String {
    match v {
        Value::Lgl(x) => encode_logical(x.data[i], 0, "NA"),
        Value::Int(x) => encode_integer(x.data[i], 0, "NA"),
        Value::Real(x) => {
            let xi = x.data[i];
            let (w, d, e) = format_real(&[xi], 15, 0, 0, 2);
            encode_real0(xi, w, d, e, "NA")
        }
        Value::Cplx(x) => {
            let z = x.data[i];
            let (r, im) = format_complex(&[z], 15, 0, 0, 2);
            encode_complex(z, r, im, "NA")
        }
        Value::Str(x) => match &x.data[i] {
            None => "NA".into(),
            Some(s) => robigo_syntax::dump::encode_string(s, '"'),
        },
        Value::Raw(x) => format!("{:02x}", x.data[i]),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt1(x: f64) -> String {
        let (w, d, e) = format_real(&[x], 7, 0, 0, 2);
        encode_real0(x, w, d, e, "NA")
    }

    #[test]
    fn reals() {
        assert_eq!(fmt1(1.0), "1");
        assert_eq!(fmt1(0.1 + 0.2), "0.3");
        assert_eq!(fmt1(1.0 / 3.0), "0.3333333");
        assert_eq!(fmt1(1e5), "1e+05");
        assert_eq!(fmt1(123456.0), "123456");
        assert_eq!(fmt1(1e-20), "1e-20");
        assert_eq!(fmt1(3.14159265), "3.141593");
        assert_eq!(fmt1(-2.5), "-2.5");
        assert_eq!(fmt1(100.0), "100");
        assert_eq!(fmt1(1234567.0), "1234567");
        assert_eq!(fmt1(0.0001), "1e-04");
        assert_eq!(fmt1(0.001), "0.001");
    }

    #[test]
    fn vectors() {
        let v = [1.0, 2.5, 10.0];
        let (w, d, e) = format_real(&v, 7, 0, 0, 2);
        assert_eq!((w, d, e), (4, 1, 0));
    }

    #[test]
    fn as_character() {
        assert_eq!(string_from_real(0.1).as_deref(), Some("0.1"));
        assert_eq!(string_from_real(1.0 / 3.0).as_deref(), Some("0.333333333333333"));
        assert_eq!(string_from_real(1e5).as_deref(), Some("1e+05"));
        assert_eq!(string_from_real(123456.0).as_deref(), Some("123456"));
        assert_eq!(string_from_real(1e15).as_deref(), Some("1e+15"));
    }
}
