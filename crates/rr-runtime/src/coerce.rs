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

pub fn str_to_real(s: &str) -> Option<f64> {
    let t = s.trim_matches(|c: char| c.is_ascii_whitespace());
    if t.is_empty() {
        return None;
    }
    let (sign, body) = match t.as_bytes()[0] {
        b'-' => (-1.0, &t[1..]),
        b'+' => (1.0, &t[1..]),
        _ => (1.0, t),
    };
    let lower = body.to_ascii_lowercase();
    if lower == "nan" {
        return Some(f64::NAN);
    }
    if lower == "inf" || lower == "infinity" {
        return Some(sign * f64::INFINITY);
    }
    if lower.starts_with("0x") {
        let hex = &body[2..];
        let (mant, exp) = match hex.find(['p', 'P']) {
            Some(i) => (&hex[..i], Some(&hex[i + 1..])),
            None => (hex, None),
        };
        let (int_part, frac_part) = match mant.find('.') {
            Some(i) => (&mant[..i], &mant[i + 1..]),
            None => (mant, ""),
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return None;
        }
        let mut v = 0.0f64;
        for c in int_part.chars().chain(frac_part.chars()) {
            v = v * 16.0 + c.to_digit(16)? as f64;
        }
        let mut e: i32 = -(4 * frac_part.len() as i32);
        if let Some(exp) = exp {
            e += exp.parse::<i32>().ok()?;
        }
        return Some(sign * v * 2f64.powi(e));
    }
    // Decimal: digits [. digits] [e[+-]digits]
    let b = body.as_bytes();
    let mut i = 0;
    let mut nd = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        nd += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            nd += 1;
        }
    }
    if nd == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let save = i;
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let ds = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == ds {
            i = save;
            // R accepts "1e" as 1? R_strtod backs out the exponent.
            let _ = i;
            return None;
        }
    }
    if i != b.len() {
        return None;
    }
    body.parse::<f64>().ok().map(|v| sign * v)
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
    if x == NA_INTEGER {
        NA_REAL
    } else {
        x as f64
    }
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
        None
    } else {
        rstr(&x.to_string())
    }
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
        Value::List(x) => x
            .data
            .iter()
            .map(|e| {
                if e.len() == 1 && e.is_atomic() {
                    to_lgl(e, w)[0]
                } else {
                    NA_LOGICAL
                }
            })
            .collect(),
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
                if z.is_na() {
                    NA_INTEGER
                } else {
                    if z.im != 0.0 {
                        w.imag_discarded = true;
                    }
                    let mut warn = false;
                    let r = real_to_int(z.re, &mut warn);
                    if warn {
                        w.int_range = true;
                    }
                    r
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
        Value::List(x) => x
            .data
            .iter()
            .map(|e| if e.len() == 1 && e.is_atomic() { to_int(e, w)[0] } else { NA_INTEGER })
            .collect(),
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
        Value::List(x) => x
            .data
            .iter()
            .map(|e| if e.len() == 1 && e.is_atomic() { to_real(e, w)[0] } else { NA_REAL })
            .collect(),
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
        Value::List(x) => x
            .data
            .iter()
            .map(|e| if e.len() == 1 && e.is_atomic() { to_cplx(e, w)[0] } else { Complex::NA })
            .collect(),
        _ => to_real(v, w)
            .into_iter()
            .map(|d| if is_na_real(d) { Complex::NA } else { Complex::new(d, 0.0) })
            .collect(),
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
        _ => to_int(v, w)
            .into_iter()
            .map(|i| {
                if i == NA_INTEGER || !(0..=255).contains(&i) {
                    w.na_introduced = true;
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
        to_int(&first_elt(v), &mut w)[0]
    } else {
        NA_INTEGER
    }
}

pub fn as_real(v: &Value) -> f64 {
    if v.is_atomic() && !v.is_empty() {
        let mut w = CoerceWarnings::default();
        to_real(&first_elt(v), &mut w)[0]
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
