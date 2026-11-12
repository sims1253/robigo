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
    pub out_dec: String,
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
            out_dec: ".".into(),
        }
    }
}

use crate::nmath::ldouble::LDouble;

/// The largest power of ten in `tbl`.
const KP_MAX: i32 = 27;

/// GNU R's `tbl[]`: declared `long double`, but initialized with `double`
/// literals, so 1e23 to 1e27 are the nearest doubles, not exact powers.
const TBL: [f64; 28] = [
    1e00, 1e01, 1e02, 1e03, 1e04, 1e05, 1e06, 1e07, 1e08, 1e09, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18,
    1e19, 1e20, 1e21, 1e22, 1e23, 1e24, 1e25, 1e26, 1e27,
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
    if n >= w { s.to_string() } else { format!("{}{}", " ".repeat(w - n), s) }
}

/// `EncodeReal0`.
pub fn encode_real0(x: f64, w: usize, d: usize, e: usize, na: &str) -> String {
    encode_real_dec(x, w, d, e, na, ".")
}

pub fn encode_real_dec(x: f64, w: usize, d: usize, e: usize, na: &str, dec: &str) -> String {
    let x = if x == 0.0 { 0.0 } else { x };
    let mut s = if !x.is_finite() {
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
    if dec != "." && !dec.is_empty() {
        s = s.replace('.', dec);
    }
    pad_left(&s, w)
}

/// `EncodeRealDrop0`: like [`encode_real0`] but dropping trailing zeros.
#[inline(always)]
pub fn encode_real_drop0(x: f64, w: usize, d: usize, e: usize, na: &str) -> String {
    let mut s = encode_real0(x, w, d, e, na);
    drop_trailing_zeros_in(&mut s);
    s
}

/// [`encode_real_drop0`] with R's `OutDec` separator: `as.character()` of a
/// number uses the `options("OutDec")` character (format.c:EncodeReal).
pub fn encode_real_drop0_dec(x: f64, w: usize, d: usize, e: usize, na: &str, dec: OutDec) -> String {
    // R: printutils.c:EncodeRealDrop0 drops trailing zeros before replacing
    // "." by `dec`. Written out rather than wrapping `encode_real_drop0` so
    // the hot default path keeps its single-caller inlining.
    let mut s = encode_real0(x, w, d, e, na);
    drop_trailing_zeros_in(&mut s);
    if !dec.is_default() {
        s = s.replace('.', dec.as_str());
    }
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
#[inline(always)]
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
    let dec = out_dec();
    if dec.is_default() { string_from_real_default(x) } else { string_from_real_dec(x, dec) }
}

/// [`string_from_real`] with the default `OutDec`: identical to R's
/// `coerce.c:StringFromReal` before `OutDec` is consulted.
pub fn string_from_real_default(x: f64) -> RStr {
    if is_na_real(x) {
        return None;
    }
    let (w, d, e) = format_real(&[x], 15, 0, 0, 2);
    rstr(&encode_real_drop0(x, w, d, e, "NA"))
}

/// [`string_from_real`] under a non-default `OutDec`; off the hot path.
#[cold]
#[inline(never)]
pub fn string_from_real_dec(x: f64, dec: OutDec) -> RStr {
    if is_na_real(x) {
        return None;
    }
    let (w, d, e) = format_real(&[x], 15, 0, 0, 2);
    rstr(&encode_real_drop0_dec(x, w, d, e, "NA", dec))
}

/// R's global `OutDec`: `static char sdec[11]` in options.c, which
/// `EncodeReal0` and `EncodeComplex` read. It is copied by value, like the
/// C global, so reading it costs a thread-local load rather than an
/// allocation on the number-to-string hot path.
#[derive(Clone, Copy)]
pub struct OutDec {
    len: u8,
    bytes: [u8; 10],
}

impl OutDec {
    const DEFAULT: OutDec = OutDec { len: 1, bytes: [b'.', 0, 0, 0, 0, 0, 0, 0, 0, 0] };

    /// `strcmp(OutDec, ".") == 0`: the common case.
    pub fn is_default(&self) -> bool {
        self.len == 1 && self.bytes[0] == b'.'
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or(".")
    }
}

thread_local! {
    static OUT_DEC: std::cell::Cell<OutDec> = const { std::cell::Cell::new(OutDec::DEFAULT) };
}

/// `options(OutDec =)`: the character `EncodeReal0`/`EncodeComplex` use.
// R: options.c:do_options — `strncpy(sdec, CHAR(STRING_ELT(argi, 0)), 10)`.
pub fn set_out_dec(s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(10);
    let mut bytes = [0u8; 10];
    bytes[..n].copy_from_slice(&b[..n]);
    OUT_DEC.with(|d| d.set(OutDec { len: n as u8, bytes }));
}

/// The current `OutDec` (default `"."`).
pub fn out_dec() -> OutDec {
    OUT_DEC.with(|d| d.get())
}

/// The number of digits of `n`; 0 for 0.
// R: printutils.c:IndexWidth
pub fn index_width(n: usize) -> usize {
    ((n as f64 + 0.5).log10() + 1.0) as usize
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
    if x == NA_INTEGER { pad_left(na, w) } else { pad_left(&x.to_string(), w) }
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
    encode_complex_dec(z, wr, wi, na, ".")
}

pub fn encode_complex_dec(
    z: Complex,
    wr: (usize, usize, usize),
    wi: (usize, usize, usize),
    na: &str,
    dec: &str,
) -> String {
    if is_na_real(z.re) || is_na_real(z.im) {
        return pad_left(na, wr.0 + wi.0 + 2);
    }
    let re = if z.re == 0.0 { 0.0 } else { z.re };
    let mut imv = if z.im == 0.0 { 0.0 } else { z.im };
    let re_s = encode_real_dec(re, wr.0, wr.1, wr.2, na, dec);
    let mut neg = imv < 0.0;
    if neg {
        imv = -imv;
    }
    let im_s = encode_real_dec(imv, wi.0, wi.1, wi.2, na, dec);
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
    let q = quote.unwrap_or('"') as u8;
    let mut out = Vec::new();
    robigo_syntax::dump::encode_string(s.as_bytes(), q, &mut out);
    if quote.is_none() {
        // Strip the quotes and un-escape `\"`.
        let inner = String::from_utf8_lossy(&out[1..out.len() - 1]).into_owned();
        return inner.replace("\\\"", "\"");
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// R: printutils.c:EncodeString without padding.
#[inline]
pub fn escape_rstring(s: &RString, quote: Option<char>) -> String {
    if crate::locale::utf8locale() && !s.has_foreign_bytes() {
        return escape_string(s, quote);
    }
    encode_string_slow(s, quote, false).0
}

/// `Rstrlen`: the width R reserves for `s` printed with `quote`, before
/// the quotes themselves.
#[inline]
pub fn rstrlen(s: &RString, quote: Option<char>) -> usize {
    if crate::locale::utf8locale() && !s.has_foreign_bytes() {
        return str_width(&escape_string(s, quote)) - if quote.is_some() { 2 } else { 0 };
    }
    rstrlen_slow(s, quote)
}

/// How [`rstrwid`] reads its bytes (R's `ienc` argument).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ienc {
    Native,
    Utf8,
    Bytes,
}

// R: printutils.c:Rstrlen
#[cold]
#[inline(never)]
fn rstrlen_slow(s: &RString, quote: Option<char>) -> usize {
    let q = quote.map(|c| c as u8);
    match s.encoding() {
        Encoding::Utf8 | Encoding::Utf8Invalid => rstrwid(&s.raw_bytes(), Ienc::Utf8, q),
        Encoding::Bytes => rstrwid(&s.raw_bytes(), Ienc::Bytes, q),
        _ => rstrwid(&translate_or_raw(s), Ienc::Native, q),
    }
}

/// translateChar for printing, which never sees "bytes" strings; a failed
/// conversion (no iconv for the locale) prints the bytes as they are.
fn translate_or_raw(s: &RString) -> std::borrow::Cow<'_, [u8]> {
    crate::locale::translate_char(s).unwrap_or_else(|_| s.raw_bytes())
}

/// The character at the start of `p` the way EncodeString and Rstrwid
/// read it: UTF-8 for strings marked so, `mbrtowc` in other multibyte
/// locales. `None` is an invalid sequence.
fn next_wc(p: &[u8], utf8: bool, mb: &mut crate::sys::MbDecoder) -> Option<(u32, usize)> {
    if utf8 {
        let len = match p[0] {
            0..0x80 => 1,
            0xc0..0xe0 => 2,
            0xe0..0xf0 => 3,
            0xf0..0xf8 => 4,
            _ => return None,
        };
        let c = std::str::from_utf8(p.get(..len)?).ok()?.chars().next()?;
        Some((c as u32, len))
    } else {
        match mb.next(p)? {
            (wc, 0) => Some((wc, 1)),
            (wc, n) if wc <= 0x10FFFF => Some((wc, n)),
            _ => None,
        }
    }
}

/// The width of `p` printed with `quote`, in a locale that is not UTF-8
/// or for bytes strings (a UTF-8 locale measures what [`escape_string`]
/// prints, which is the same count).
// R: printutils.c:Rstrwid
fn rstrwid(p: &[u8], ienc: Ienc, quote: Option<u8>) -> usize {
    if ienc == Ienc::Bytes {
        return p.iter().map(|&k| if (0x20..0x80).contains(&k) { 1 } else { 4 }).sum();
    }
    if crate::locale::utf8locale() {
        let mut out = Vec::new();
        encode_utf8_locale(p, quote, &mut out);
        return str_width(&String::from_utf8_lossy(&out)) - if quote.is_some() { 2 } else { 0 };
    }
    let mut len = 0;
    let mut i = 0;
    if crate::locale::mbcslocale() || ienc == Ienc::Utf8 {
        let mut mb = crate::sys::MbDecoder::new();
        while i < p.len() {
            match next_wc(&p[i..], ienc == Ienc::Utf8, &mut mb) {
                Some((k, res)) => {
                    if (0x20..0x7f).contains(&k) && crate::sys::is_wprint(k) {
                        len += match k as u8 {
                            b'\\' => 2,
                            b'\'' | b'"' | b'`' if quote == Some(p[i]) => 2,
                            _ => 1,
                        };
                        i += 1;
                    } else if k < 0x80 {
                        len += match k as u8 {
                            0x07 | 0x08 | 0x0c | b'\n' | b'\r' | b'\t' | 0x0b | 0 => 2,
                            // print in octal
                            _ => 4,
                        };
                        i += 1;
                    } else {
                        len += if crate::sys::is_wprint(k) {
                            crate::sys::wc_width(k).max(0) as usize
                        } else if k > 0xffff {
                            10
                        } else {
                            6
                        };
                        i += res;
                    }
                }
                // invalid char
                None => {
                    len += 4;
                    i += 1;
                }
            }
        }
    } else {
        // not MBCS nor marked as UTF-8
        for &c in p {
            len += if c < 0x80 {
                if crate::sys::is_print(c) {
                    match c {
                        b'\\' => 2,
                        b'\'' | b'"' | b'`' if quote == Some(c) => 2,
                        _ => 1,
                    }
                } else {
                    match c {
                        0x07 | 0x08 | 0x0c | b'\n' | b'\r' | b'\t' | 0x0b | 0 => 2,
                        // print in octal
                        _ => 4,
                    }
                }
            } else if crate::sys::is_print(c) {
                1
            } else {
                4
            };
        }
    }
    len
}

/// EncodeString's text for `s` without the padding, and the width it pads
/// by. `use_utf8` keeps strings marked UTF-8 in UTF-8 (`encodeString`'s
/// `w - 1000000`).
// R: printutils.c:EncodeString
#[cold]
#[inline(never)]
pub(crate) fn encode_string_slow(s: &RString, quote: Option<char>, use_utf8: bool) -> (String, usize) {
    use std::borrow::Cow;
    let q = quote.map(|c| c as u8);
    let (p, utf8, i, cnt): (Cow<[u8]>, bool, usize, usize) = if s.is_bytes() {
        let raw = s.raw_bytes();
        let text = crate::locale::bytes_escaped(&raw);
        // EncodeString counts a `"` once more, and then reads that many
        // characters: past the end, where it finds the terminator.
        let cnt = text.len() + if q.is_some() { raw.iter().filter(|&&k| k == b'"').count() } else { 0 };
        (Cow::Owned(text.into_bytes()), false, cnt, cnt)
    } else if use_utf8 && matches!(s.encoding(), Encoding::Utf8 | Encoding::Utf8Invalid) {
        let p = s.raw_bytes();
        let n = p.len();
        (p, true, rstrlen_slow(s, quote), n)
    } else {
        let p = translate_or_raw(s);
        let i = if crate::locale::translates(s) { rstrwid(&p, Ienc::Native, q) } else { rstrlen_slow(s, quote) };
        let n = p.len();
        (p, false, i, n)
    };
    let mut out: Vec<u8> = Vec::with_capacity(cnt + 2);
    if crate::locale::utf8locale() && cnt == p.len() {
        encode_utf8_locale(&p, q, &mut out);
        return (String::from_utf8_lossy(&out).into_owned(), i);
    }
    // What EncodeString reads past the end of `p`: the terminator.
    let at = |k: usize| p.get(k).copied().unwrap_or(0);
    if let Some(q) = q {
        out.push(q);
    }
    let mut k = 0;
    if crate::locale::mbcslocale() || utf8 {
        let mut mb = crate::sys::MbDecoder::new();
        while k < cnt {
            let decoded = if k < p.len() { next_wc(&p[k..], utf8, &mut mb) } else { Some((0, 1)) };
            match decoded {
                Some((wc, res)) => {
                    if (0x20..0x7f).contains(&wc) && crate::sys::is_wprint(wc) {
                        let c = wc as u8;
                        match c {
                            b'\\' => out.extend_from_slice(b"\\\\"),
                            b'\'' | b'"' | b'`' => {
                                if q == Some(c) {
                                    out.push(b'\\');
                                }
                                out.push(c);
                            }
                            _ => out.extend_from_slice(&p[k..k + res]),
                        }
                        k += 1;
                    } else if wc < 0x80 {
                        push_ansi_escape(&mut out, wc as u8);
                        k += 1;
                    } else {
                        if crate::sys::is_wprint(wc) {
                            out.extend_from_slice(&p[k..k + res]);
                        } else if wc > 0xffff {
                            out.extend_from_slice(format!("\\U{{{wc:06x}}}").as_bytes());
                        } else {
                            out.extend_from_slice(format!("\\u{wc:04x}").as_bytes());
                        }
                        k += res;
                    }
                }
                // invalid char
                None => {
                    out.extend_from_slice(format!("\\x{:02x}", at(k)).as_bytes());
                    k += 1;
                }
            }
        }
    } else {
        while k < cnt {
            let c = at(k);
            if c < 0x80 {
                // Windows has \t as printable
                if c != b'\t' && crate::sys::is_print(c) {
                    match c {
                        b'\\' => out.extend_from_slice(b"\\\\"),
                        b'\'' | b'"' | b'`' => {
                            if q == Some(c) {
                                out.push(b'\\');
                            }
                            out.push(c);
                        }
                        _ => out.push(c),
                    }
                } else {
                    push_ansi_escape(&mut out, c);
                }
            } else if !crate::sys::is_print(c) {
                // print in octal
                out.extend_from_slice(format!("\\{c:03o}").as_bytes());
            } else {
                out.push(c);
            }
            k += 1;
        }
    }
    if let Some(q) = q {
        out.push(q);
    }
    (crate::locale::native_to_display(&out), i)
}

/// EncodeString's multibyte loop in a UTF-8 locale, whose `iswprint`
/// [`escape_string`]'s table holds: with `quote` (when one of `'`, `"` and
/// `` ` ``, which alone are escaped) around the text.
fn encode_utf8_locale(p: &[u8], quote: Option<u8>, out: &mut Vec<u8>) {
    match quote {
        Some(q @ (b'"' | b'\'' | b'`')) => robigo_syntax::dump::encode_string(p, q, out),
        Some(q) => {
            out.push(q);
            robigo_syntax::dump::encode_bare(p, out);
            out.push(q);
        }
        None => robigo_syntax::dump::encode_bare(p, out),
    }
}

/// `encodeString`'s element: EncodeString padded to `w`, for strings a
/// UTF-8 locale cannot print as they are or any string in another locale.
/// Strings marked UTF-8 stay UTF-8 (R passes `w - 1000000`); the others
/// come back in the native encoding.
// R: util.c:do_encodeString
pub fn encode_string_marked(s: &RString, w: usize, quote: Option<char>, justify: Justify) -> RString {
    let utf8 = matches!(s.encoding(), Encoding::Utf8 | Encoding::Utf8Invalid);
    let (body, i) = encode_string_slow(s, quote, utf8);
    let text = pad_encoded(body, i + if quote.is_some() { 2 } else { 0 }, w, justify);
    let bytes = crate::locale::output_bytes(&text);
    RString::from_bytes(&bytes, if utf8 { Encoding::Utf8 } else { Encoding::Native })
}

/// EncodeString's ANSI escapes, and octal for the other controls.
fn push_ansi_escape(out: &mut Vec<u8>, c: u8) {
    let e = match c {
        0x07 => b'a',
        0x08 => b'b',
        0x0c => b'f',
        b'\n' => b'n',
        b'\r' => b'r',
        b'\t' => b't',
        0x0b => b'v',
        0 => b'0',
        _ => {
            // print in octal
            out.extend_from_slice(format!("\\{c:03o}").as_bytes());
            return;
        }
    };
    out.extend_from_slice(&[b'\\', e]);
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
        Some(s) if !crate::locale::utf8locale() || s.has_foreign_bytes() => {
            let (body, i) = encode_string_slow(s, quote, false);
            return pad_encoded(body, i + if quote.is_some() { 2 } else { 0 }, w, justify);
        }
        Some(s) => escape_rstring(s, quote),
    };
    let bw = str_width(&body);
    pad_encoded(body, bw, w, justify)
}

/// EncodeString's padding of `body`, `bw` wide, to `w`.
fn pad_encoded(body: String, bw: usize, w: usize, justify: Justify) -> String {
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
            Some(s) => rstrlen(s, quote) + if quote.is_some() { 2 } else { 0 },
        };
        w = w.max(l);
    }
    w
}

/// Element encoding used by deparse (digits = 15, no padding).
/// C's `snprintf("%a", x)` for a finite double (glibc): the exact
/// hexadecimal significand with trailing zero nibbles removed.
pub fn encode_hex_real(x: f64) -> String {
    let bits = x.to_bits();
    let sign = if bits >> 63 != 0 { "-" } else { "" };
    let exp_bits = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    if exp_bits == 0 && frac == 0 {
        return format!("{sign}0x0p+0");
    }
    // Subnormals print with a leading 0 and the exponent fixed at -1022.
    let (lead, exp) = if exp_bits == 0 { (0, -1022) } else { (1, exp_bits - 1023) };
    let mut digits = String::new();
    for k in 0..13 {
        let d = (frac >> (48 - 4 * k)) & 0xf;
        digits.push(char::from_digit(d as u32, 16).unwrap());
    }
    let digits = digits.trim_end_matches('0');
    let mut s = format!("{sign}0x{lead}");
    if !digits.is_empty() {
        s.push('.');
        s.push_str(digits);
    }
    s.push_str(&format!("p{exp:+}"));
    s
}

/// R's `%a + %ai` for a finite complex element (deparse.c).
pub fn encode_hex_complex(z: Complex) -> String {
    format!("{} + {}i", encode_hex_real(z.re), encode_hex_real(z.im))
}

/// C's `%+.17g` for a finite double (deparse's digits17 complex).
pub fn encode_digits17_signed(x: f64) -> String {
    let s = robigo_syntax::cfmt::fmt_g(x, 17, false, false);
    if s.starts_with('-') { s } else { format!("+{s}") }
}

/// `deparse.c:EncodeNonFiniteComplexElement`: a complex literal whose real
/// part is NaN or whose imaginary part is not finite cannot be written as
/// `a+bi`, so deparse spells it out as `complex(real=..., imaginary=...)`.
pub fn encode_nonfinite_complex(z: Complex) -> String {
    let (r, im) = format_complex(&[z], 15, 0, 0, 2);
    let re = encode_real0(z.re, r.0, r.1, r.2, "NA");
    let imag = encode_real0(z.im, im.0, im.1, im.2, "NA");
    format!("complex(real={re}, imaginary={imag})")
}

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
            Some(s) => escape_rstring(s, Some('"')),
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
        let (w, d, e) = format_real(&[1.5], 7, 0, 0, 2);
        assert_eq!(encode_real_dec(1.5, w, d, e, "NA", ","), "1,5");
    }

    #[test]
    fn nonfinite_complex() {
        assert_eq!(encode_nonfinite_complex(Complex::new(f64::NAN, 0.0)), "complex(real=NaN, imaginary=0)");
        assert_eq!(encode_nonfinite_complex(Complex::new(f64::INFINITY, 4.0)), "complex(real=Inf, imaginary=4)");
        assert_eq!(encode_nonfinite_complex(Complex::new(3.0, f64::NEG_INFINITY)), "complex(real=3, imaginary=-Inf)");
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
