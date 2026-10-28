//! Type coercion between vector types (GNU R's coerce.c).

use std::rc::Rc;

use crate::error::{Flow, RResult};
use crate::format::string_from_real;
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

/// Parse a number the way `as.numeric("...")` does (R_strtod + checks for
/// trailing garbage). `None` means NA (with a coercion warning).
/// R's `isBlankString`.
pub fn is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// Parse a number as `as.numeric("...")` does (R's `RealFromString`):
/// [`r_strtod5`] followed by only blanks; `None` means NA with a warning.
pub fn str_to_real(s: &str) -> Option<f64> {
    let (x, used) = r_strtod5(s.as_bytes(), b'.', false, 0);
    if is_blank(&s[used..]) { Some(x) } else { None }
}

/// R's number parser: digits accumulated and scaled in long double, so
/// results (including double rounding) are GNU R's. Returns the value and
/// the number of bytes consumed (0 if nothing was parsed). `dec` is the
/// decimal point, `na` accepts "NA", `exact` is type.convert's
/// `numerals` handling (0: allow loss, 1: NA on loss, NA_LOGICAL: warn).
// R: util.c:R_strtod5
pub fn r_strtod5(s: &[u8], dec: u8, na: bool, exact: i32) -> (f64, usize) {
    use crate::nmath::ldouble::LDouble as LD;
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
    if starts_ci(p, b"NaN") {
        return (sign * f64::NAN, p + 3);
    } else if starts_ci(p, b"infinity") {
        return (sign * f64::INFINITY, p + 8);
    } else if starts_ci(p, b"Inf") {
        return (sign * f64::INFINITY, p + 3);
    }
    let back_out = (NA_REAL, 0usize);
    let mut ans = LD::from_f64(0.0);
    let mut expn: i64 = 0;
    let max_exp_prefix: i64 = 9999;
    let exact_clause = |ans: LD| -> Option<bool> {
        // Some(true): back out with NA; Some(false): warn.
        if exact != 0 && ans > LD::from_f64(f64::from_bits(0x433f_ffff_ffff_ffff)) {
            Some(exact != NA_LOGICAL)
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
    let mut digit = |ans: &mut LD, d: u8| {
        if !big && acc < (i64::MAX as u64 - 9) / 10 {
            acc = acc * 10 + d as u64;
        } else {
            if !big {
                *ans = LD::from_i64(acc as i64);
                big = true;
            }
            *ans = *ans * 10.0 + d as f64;
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
    if !big {
        ans = LD::from_i64(acc as i64);
    }
    if ndigits == 0 {
        return back_out;
    }
    match exact_clause(ans) {
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
    if expn + ndigits < -300 {
        for _ in 0..ndigits {
            ans /= 10.0;
        }
        expn += ndigits;
    }
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
    if ans > LD::from_f64(f64::MAX) {
        return (sign * f64::INFINITY, p);
    }
    (sign * ans.to_f64(), p)
}

pub fn str_to_logical(s: &str) -> Lgl {
    match s {
        "TRUE" | "true" | "T" | "True" => TRUE,
        "FALSE" | "false" | "F" | "False" => FALSE,
        _ => NA_LOGICAL,
    }
}

pub fn real_to_int(x: f64, warn: &mut bool) -> i32 {
    if x.is_nan() {
        NA_INTEGER
    } else if x >= i32::MAX as f64 + 1.0 || x <= i32::MIN as f64 {
        *warn = true;
        NA_INTEGER
    } else {
        x.trunc() as i32
    }
}

pub fn int_to_real(x: i32) -> f64 {
    if x == NA_INTEGER { NA_REAL } else { x as f64 }
}

pub fn lgl_to_str(x: Lgl) -> RStr {
    if x == NA_LOGICAL {
        None
    } else if x != 0 {
        rstr("TRUE")
    } else {
        rstr("FALSE")
    }
}

pub fn int_to_str(x: i32) -> RStr {
    if x == NA_INTEGER {
        return None;
    }
    // Small non-negative integers (row names, indices) share their strings.
    const CACHED: i32 = 1 << 16;
    thread_local! {
        static SMALL: std::cell::RefCell<Vec<RStr>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    if (0..CACHED).contains(&x) {
        return SMALL.with(|c| {
            let mut c = c.borrow_mut();
            if c.is_empty() {
                c.resize(CACHED as usize, None);
            }
            c[x as usize].get_or_insert_with(|| int_to_str_new(x).unwrap()).clone().into()
        });
    }
    int_to_str_new(x)
}

fn int_to_str_new(x: i32) -> RStr {
    // Digits written directly (no formatting machinery, one allocation).
    let mut buf = [0u8; 12];
    let mut i = buf.len();
    let mut v = (x as i64).unsigned_abs();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if x < 0 {
        i -= 1;
        buf[i] = b'-';
    }
    // SAFETY: ASCII digits and '-'.
    rstr(unsafe { std::str::from_utf8_unchecked(&buf[i..]) })
}

pub fn cplx_to_str(z: Complex) -> RStr {
    if is_na_real(z.re) || is_na_real(z.im) {
        return None;
    }
    let (r, i) = crate::format::format_complex(&[z], 15, 0, 0, 2);
    let s = crate::format::encode_complex(z, r, i, "NA");
    // as.character drops trailing zeros in each part.
    Some(Rc::from(s.as_str()))
}

#[derive(Default)]
pub struct CoerceWarnings {
    pub na_introduced: bool,
    pub int_range: bool,
    pub imag_discarded: bool,
    /// WARN_RAW: out-of-range values become 0 in raw coercion.
    pub raw_out_of_range: bool,
}

impl CoerceWarnings {
    /// Emit as R's coerceVector does: `warning()` against the context call.
    pub fn emit(&self, interp: &mut Interp) -> RResult<()> {
        if self.na_introduced {
            interp.warning("NAs introduced by coercion")?;
        }
        if self.int_range {
            interp.warning("NAs introduced by coercion to integer range")?;
        }
        if self.imag_discarded {
            interp.warning("imaginary parts discarded in coercion")?;
        }
        if self.raw_out_of_range {
            interp.warning("out-of-range values treated as 0 in coercion to raw")?;
        }
        Ok(())
    }
}

/// Coerce an atomic (or list) vector to logical values.
pub fn to_lgl(v: &Value, w: &mut CoerceWarnings) -> Vec<Lgl> {
    match v {
        Value::Null => vec![],
        Value::Lgl(x) => x.data.clone(),
        Value::Int(x) => x.data.iter().map(|&i| if i == NA_INTEGER { NA_LOGICAL } else { (i != 0) as Lgl }).collect(),
        Value::Real(x) => x.data.iter().map(|&d| if d.is_nan() { NA_LOGICAL } else { (d != 0.0) as Lgl }).collect(),
        Value::Cplx(x) => {
            x.data.iter().map(|z| if z.is_na() { NA_LOGICAL } else { (z.re != 0.0 || z.im != 0.0) as Lgl }).collect()
        }
        Value::Str(x) => x.data.iter().map(|s| s.as_deref().map(str_to_logical).unwrap_or(NA_LOGICAL)).collect(),
        Value::Raw(x) => x.data.iter().map(|&b| (b != 0) as Lgl).collect(),
        Value::List(x) => {
            x.data.iter().map(|e| if e.len() == 1 && e.is_atomic() { to_lgl(e, w)[0] } else { NA_LOGICAL }).collect()
        }
        _ => vec![],
    }
}

pub fn to_int(v: &Value, w: &mut CoerceWarnings) -> Vec<i32> {
    match v {
        Value::Null => vec![],
        Value::Lgl(x) => x.data.clone(),
        Value::Int(x) => x.data.clone(),
        Value::Real(x) => x
            .data
            .iter()
            .map(|&d| {
                let mut warn = false;
                let r = real_to_int(d, &mut warn);
                if warn {
                    w.int_range = true;
                }
                r
            })
            .collect(),
        Value::Cplx(x) => x
            .data
            .iter()
            .map(|z| {
                // R: coerce.c:IntegerFromComplex — a NaN in either part
                // gives NA without a warning; the range check comes before
                // the imaginary warning.
                if z.re.is_nan() || z.im.is_nan() {
                    NA_INTEGER
                } else if z.re > i32::MAX as f64 + 1.0 || z.re <= i32::MIN as f64 {
                    w.int_range = true;
                    NA_INTEGER
                } else {
                    if z.im != 0.0 {
                        w.imag_discarded = true;
                    }
                    z.re as i32
                }
            })
            .collect(),
        Value::Str(x) => x
            .data
            .iter()
            .map(|s| match s {
                None => NA_INTEGER,
                Some(s) => match str_to_real(s) {
                    Some(d) => {
                        if d.is_nan() {
                            NA_INTEGER
                        } else if d >= i32::MAX as f64 + 1.0 || d <= i32::MIN as f64 {
                            w.int_range = true;
                            NA_INTEGER
                        } else {
                            d.trunc() as i32
                        }
                    }
                    None => {
                        // R: String2Real etc.: blank strings give NA silently.
                        if !is_blank(s) {
                            w.na_introduced = true;
                        }
                        NA_INTEGER
                    }
                },
            })
            .collect(),
        Value::Raw(x) => x.data.iter().map(|&b| b as i32).collect(),
        Value::List(x) => {
            x.data.iter().map(|e| if e.len() == 1 && e.is_atomic() { to_int(e, w)[0] } else { NA_INTEGER }).collect()
        }
        _ => vec![],
    }
}

pub fn to_real(v: &Value, w: &mut CoerceWarnings) -> Vec<f64> {
    match v {
        Value::Null => vec![],
        Value::Lgl(x) | Value::Int(x) => x.data.iter().map(|&i| int_to_real(i)).collect(),
        Value::Real(x) => x.data.clone(),
        Value::Cplx(x) => x
            .data
            .iter()
            .map(|z| {
                if z.is_na() {
                    NA_REAL
                } else {
                    if z.im != 0.0 {
                        w.imag_discarded = true;
                    }
                    z.re
                }
            })
            .collect(),
        Value::Str(x) => x
            .data
            .iter()
            .map(|s| match s {
                None => NA_REAL,
                Some(s) => match str_to_real(s) {
                    Some(d) => d,
                    None => {
                        // R: String2Real etc.: blank strings give NA silently.
                        if !is_blank(s) {
                            w.na_introduced = true;
                        }
                        NA_REAL
                    }
                },
            })
            .collect(),
        Value::Raw(x) => x.data.iter().map(|&b| b as f64).collect(),
        Value::List(x) => {
            x.data.iter().map(|e| if e.len() == 1 && e.is_atomic() { to_real(e, w)[0] } else { NA_REAL }).collect()
        }
        _ => vec![],
    }
}

pub fn to_cplx(v: &Value, w: &mut CoerceWarnings) -> Vec<Complex> {
    match v {
        Value::Cplx(x) => x.data.clone(),
        Value::Str(x) => x
            .data
            .iter()
            .map(|s| match s {
                None => Complex::NA,
                Some(s) => match parse_complex(s) {
                    Some(z) => z,
                    None => {
                        // R: String2Real etc.: blank strings give NA silently.
                        if !is_blank(s) {
                            w.na_introduced = true;
                        }
                        Complex::NA
                    }
                },
            })
            .collect(),
        Value::List(x) => {
            x.data.iter().map(|e| if e.len() == 1 && e.is_atomic() { to_cplx(e, w)[0] } else { Complex::NA }).collect()
        }
        _ => {
            // RealFromReal/Logical/Integer (R: coerce.c): the value with a
            // zero imaginary part — NA keeps its payload, so arithmetic
            // propagates it exactly like C's would.
            to_real(v, w).into_iter().map(|d| Complex::new(d, 0.0)).collect()
        }
    }
}

fn parse_complex(s: &str) -> Option<Complex> {
    let t = s.trim();
    if let Some(d) = str_to_real(t) {
        return Some(Complex::new(d, 0.0));
    }
    let body = t.strip_suffix('i')?;
    // Find the split between real and imaginary parts.
    let bytes = body.as_bytes();
    for i in (1..bytes.len()).rev() {
        if (bytes[i] == b'+' || bytes[i] == b'-') && !matches!(bytes[i - 1], b'e' | b'E') {
            let re = str_to_real(&body[..i])?;
            let im = str_to_real(&body[i..])?;
            return Some(Complex::new(re, im));
        }
    }
    let im = str_to_real(body)?;
    Some(Complex::new(0.0, im))
}

pub fn to_str(v: &Value) -> Vec<RStr> {
    match v {
        Value::Null => vec![],
        Value::Lgl(x) => x.data.iter().map(|&l| lgl_to_str(l)).collect(),
        Value::Int(x) => x.data.iter().map(|&i| int_to_str(i)).collect(),
        Value::Real(x) => x.data.iter().map(|&d| string_from_real(d)).collect(),
        Value::Cplx(x) => x.data.iter().map(|&z| cplx_to_str(z)).collect(),
        Value::Str(x) => x.data.clone(),
        Value::Raw(x) => x.data.iter().map(|b| rstr(&format!("{:02x}", b))).collect(),
        Value::List(x) | Value::Expr(x) => x
            .data
            .iter()
            .map(|e| match e {
                Value::Str(s) if s.data.len() == 1 => s.data[0].clone(),
                Value::Null => rstr("NULL"),
                e if e.is_atomic() && e.len() == 1 => to_str(e)[0].clone(),
                Value::Sym(sym) => Some(sym.name()),
                e => rstr(&crate::deparse::deparse_one_line(e)),
            })
            .collect(),
        Value::Sym(s) => vec![Some(s.name())],
        Value::Lang(_) | Value::Pairlist(_) => {
            // as.character on a call: each element deparsed.
            let items: Vec<Value> = match v {
                Value::Lang(l) => l.items().into_iter().map(|a| a.value).collect(),
                Value::Pairlist(p) => p.items.iter().map(|a| a.value.clone()).collect(),
                _ => unreachable!(),
            };
            items
                .iter()
                .map(|e| match e {
                    Value::Sym(s) => Some(s.name()),
                    Value::Str(s) if s.data.len() == 1 => s.data[0].clone(),
                    e => rstr(&crate::deparse::deparse_one_line(e)),
                })
                .collect()
        }
        _ => vec![],
    }
}

pub fn to_raw(v: &Value, w: &mut CoerceWarnings) -> Vec<u8> {
    match v {
        Value::Raw(x) => x.data.clone(),
        // R: coerce.c:coerceToRaw — NA, negative and >255 values become 0
        // with WARN_RAW (a separate warning from WARN_NA).
        _ => to_int(v, w)
            .into_iter()
            .map(|i| {
                if i == NA_INTEGER || !(0..=255).contains(&i) {
                    w.raw_out_of_range = true;
                    0
                } else {
                    i as u8
                }
            })
            .collect(),
    }
}

/// Coerce to a vector type, keeping attributes (R's `coerceVector`).
pub fn coerce_vector(v: &Value, t: VType, w: &mut CoerceWarnings) -> RResult<Value> {
    if v.vtype() == Some(t) {
        return Ok(v.clone());
    }
    // R: coerce.c:coerceVector — vectors, lists, expressions, language
    // objects and symbols coerce; environments have their own message;
    // everything else fails as a plain error against the context call.
    if let Value::Env(_) = v {
        return Err(Flow::error("environments cannot be coerced to other types"));
    }
    if !matches!(
        v,
        Value::Null
            | Value::Lgl(_)
            | Value::Int(_)
            | Value::Real(_)
            | Value::Cplx(_)
            | Value::Str(_)
            | Value::Raw(_)
            | Value::List(_)
            | Value::Expr(_)
            | Value::Pairlist(_)
            | Value::Lang(_)
            | Value::Sym(_)
    ) {
        return Err(Flow::error(format!("cannot coerce type '{}' to vector of type '{}'", v.type_name(), t.name())));
    }
    // R: coerceVectorList / coercePairList — the elements of a list (an
    // expression never qualifies: isNewList() is FALSE for EXPRSXP) must
    // all be vectors of length at most 1, except for the character
    // target, which deparses anything. The failure is a plain error.
    if matches!(v, Value::List(_) | Value::Expr(_) | Value::Pairlist(_) | Value::Lang(_))
        && !matches!(t, VType::List | VType::Expr | VType::Str)
    {
        let items: Vec<Value> = match v {
            Value::List(l) | Value::Expr(l) => l.data.clone(),
            Value::Pairlist(p) => p.items.iter().map(|a| a.value.clone()).collect(),
            Value::Lang(l) => l.items().into_iter().map(|a| a.value).collect(),
            _ => unreachable!(),
        };
        let name = match v {
            Value::Pairlist(_) => "pairlist",
            Value::Lang(_) => "language",
            _ => "list",
        };
        if matches!(v, Value::Expr(_)) || !items.iter().all(|e| e.is_atomic() && e.len() <= 1) {
            return Err(Flow::error(format!("'{}' object cannot be coerced to type '{}'", name, t.name())));
        }
        if !matches!(v, Value::List(_)) {
            // R: coercePairList — element-wise (asLogical, asInteger, ...),
            // names from the tags; lists take the element-wise path below.
            let list = Value::list(items);
            let mut out = coerce_vector(&list, t, w)?;
            out.set_attrs(None);
            if let Some(n) = v.names() {
                out.set_attr_raw(s::NAMES, n);
            }
            return Ok(out);
        }
    }
    let attrs = v.attrs().cloned();
    let mut out = match t {
        VType::Null => return Ok(Value::Null),
        VType::Lgl => Value::lgls(to_lgl(v, w)),
        VType::Int => Value::ints(to_int(v, w)),
        VType::Real => Value::reals(to_real(v, w)),
        VType::Cplx => Value::cplxs(to_cplx(v, w)),
        VType::Str => Value::strs(to_str(v)),
        VType::Raw => Value::raws(to_raw(v, w)),
        VType::List => Value::list(to_list(v)),
        VType::Expr => Value::expr_vec(to_list(v)),
    };
    if let Some(a) = attrs {
        if matches!(v, Value::Lang(_) | Value::Pairlist(_)) {
            // names of calls/pairlists already handled by to_list
        } else {
            out.set_attrs(Some(Box::new(a)));
        }
    }
    if matches!(t, VType::List | VType::Expr) {
        if let Some(n) = v.names() {
            if matches!(v, Value::Lang(_) | Value::Pairlist(_) | Value::Dots(_)) {
                out.set_attr_raw(s::NAMES, n);
            }
        }
    }
    Ok(out)
}

pub fn to_list(v: &Value) -> Vec<Value> {
    match v {
        Value::Null => vec![],
        Value::Lgl(x) => x.data.iter().map(|&e| Value::lgl_raw(e)).collect(),
        Value::Int(x) => x.data.iter().map(|&e| Value::int(e)).collect(),
        Value::Real(x) => x.data.iter().map(|&e| Value::real(e)).collect(),
        Value::Cplx(x) => x.data.iter().map(|&e| Value::cplx(e)).collect(),
        Value::Str(x) => x.data.iter().map(|e| Value::strs(vec![e.clone()])).collect(),
        Value::Raw(x) => x.data.iter().map(|&e| Value::raws(vec![e])).collect(),
        Value::List(x) | Value::Expr(x) => x.data.clone(),
        Value::Lang(l) => l.items().into_iter().map(|a| a.value).collect(),
        Value::Pairlist(p) => p.items.iter().map(|a| a.value.clone()).collect(),
        Value::Dots(d) => d.iter().map(|a| a.value.clone()).collect(),
        other => vec![other.clone()],
    }
}

/// Coerce without a call for warnings (warnings are dropped).
pub fn coerce_quiet(v: &Value, t: VType) -> Value {
    let mut w = CoerceWarnings::default();
    coerce_vector(v, t, &mut w).unwrap_or(Value::Null)
}

impl Interp {
    /// Coerce, emitting coercion warnings.
    pub fn coerce(&mut self, v: &Value, t: VType) -> RResult<Value> {
        let mut w = CoerceWarnings::default();
        let r = coerce_vector(v, t, &mut w)?;
        w.emit(self)?;
        Ok(r)
    }
}

// ----- scalar extraction (asLogical, asInteger, ...) ----------------------

pub fn as_logical(v: &Value) -> Lgl {
    if v.is_atomic() && !v.is_empty() {
        let mut w = CoerceWarnings::default();
        match v {
            Value::Lgl(x) => x.data[0],
            Value::Int(x) => match x.data[0] {
                NA_INTEGER => NA_LOGICAL,
                i => (i != 0) as Lgl,
            },
            Value::Real(x) => match x.data[0] {
                d if d.is_nan() => NA_LOGICAL,
                d => (d != 0.0) as Lgl,
            },
            Value::Str(x) => x.data[0].as_deref().map(str_to_logical).unwrap_or(NA_LOGICAL),
            _ => to_lgl(&first_elt(v), &mut w)[0],
        }
    } else {
        NA_LOGICAL
    }
}

pub fn as_integer(v: &Value) -> i32 {
    if v.is_atomic() && !v.is_empty() {
        let mut w = CoerceWarnings::default();
        match v {
            Value::Lgl(x) | Value::Int(x) => x.data[0],
            Value::Real(x) => real_to_int(x.data[0], &mut false),
            _ => to_int(&first_elt(v), &mut w)[0],
        }
    } else {
        NA_INTEGER
    }
}

pub fn as_real(v: &Value) -> f64 {
    if v.is_atomic() && !v.is_empty() {
        let mut w = CoerceWarnings::default();
        match v {
            Value::Lgl(x) | Value::Int(x) => int_to_real(x.data[0]),
            Value::Real(x) => x.data[0],
            _ => to_real(&first_elt(v), &mut w)[0],
        }
    } else {
        NA_REAL
    }
}

/// `asChar`: first element as a string.
pub fn as_string(v: &Value) -> RStr {
    match v {
        Value::Sym(s) => Some(s.name()),
        v if v.is_atomic() && !v.is_empty() => to_str(&first_elt(v))[0].clone(),
        _ => None,
    }
}

pub fn first_elt(v: &Value) -> Value {
    match v {
        Value::Lgl(x) => Value::lgl_raw(x.data[0]),
        Value::Int(x) => Value::int(x.data[0]),
        Value::Real(x) => Value::real(x.data[0]),
        Value::Cplx(x) => Value::cplx(x.data[0]),
        Value::Str(x) => Value::strs(vec![x.data[0].clone()]),
        Value::Raw(x) => Value::raws(vec![x.data[0]]),
        Value::List(x) | Value::Expr(x) => x.data[0].clone(),
        other => other.clone(),
    }
}

/// The condition of `if`/`while`: R's `asLogicalNoNA`.
pub fn as_condition(interp: &mut Interp, v: &Value, call: &Value) -> RResult<bool> {
    let len = v.len();
    if len > 1 {
        return Err(Flow::error_call(call.clone(), "the condition has length > 1"));
    }
    let cond = if len > 0 {
        match v {
            Value::Lgl(x) => x.data[0],
            Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Raw(_) => as_logical(v),
            Value::Str(_) => as_logical(v),
            _ => {
                return Err(Flow::error_call(call.clone(), "argument is not interpretable as logical"));
            }
        }
    } else {
        NA_LOGICAL
    };
    let _ = interp;
    if cond == NA_LOGICAL {
        let msg = if len == 0 {
            "argument is of length zero"
        } else if matches!(v, Value::Str(_)) {
            "argument is not interpretable as logical"
        } else {
            "missing value where TRUE/FALSE needed"
        };
        return Err(Flow::error_call(call.clone(), msg));
    }
    Ok(cond != 0)
}
