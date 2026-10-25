//! `paste`, `file.path`, `format`, `format.info` (GNU R's paste.c) and
//! `formatC` (util.c: do_formatC / str_signif).

use std::rc::Rc;

use super::character::{as_bool2, check_arity_internal};
use super::internal;
use super::sprintf::{c_format, CArg};
use crate::coerce::{as_integer, as_logical, coerce_vector, CoerceWarnings};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::format::{self, rstrlen};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("paste", do_paste, 4),
    internal("paste0", do_paste0, 3),
    internal("file.path", do_filepath, 2),
    internal("format", do_format, 9),
    internal("format.info", do_formatinfo, 3),
    internal("formatC", do_formatc, 7),
];

// ----------------------------------------------------------------------
// paste, file.path

/// Coerce the elements of paste()'s list argument to character, as
/// do_paste does: `as.character()` for objects, the print name for
/// symbols, `coerceVector()` otherwise.
fn paste_args(interp: &mut Interp, x: &Value, env: &Env, name: &str) -> RResult<Vec<Vec<RStr>>> {
    let Value::List(l) = x else {
        return Err(Flow::error("invalid first argument"));
    };
    let mut out = Vec::with_capacity(l.data.len());
    for xj in &l.data {
        let v = match xj {
            Value::Str(s) => s.data.clone(),
            Value::Sym(sy) => vec![Some(sy.name())],
            xj if xj.is_object() => {
                let call = Value::lang(Value::sym("as.character"), vec![Arg::pos(xj.clone())]);
                match interp.eval(&call, env)? {
                    Value::Str(s) => s.data.clone(),
                    _ => return Err(Flow::error(format!("non-string argument to .Internal({})", name))),
                }
            }
            xj if xj.is_vector() || xj.is_null() || matches!(xj, Value::Lang(_) | Value::Pairlist(_)) => {
                let mut w = CoerceWarnings::default();
                match coerce_vector(xj, VType::Str, &mut w)? {
                    Value::Str(s) => s.data.clone(),
                    Value::Null => vec![],
                    _ => return Err(Flow::error(format!("non-string argument to .Internal({})", name))),
                }
            }
            xj => {
                return Err(Flow::error(format!(
                    "cannot coerce type '{}' to vector of type 'character'",
                    xj.type_name()
                )));
            }
        };
        out.push(v);
    }
    Ok(out)
}

fn na_str(s: &RStr) -> &str {
    s.as_deref().unwrap_or("NA")
}

// R: paste.c:do_paste
fn paste_impl(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env, use_sep: bool) -> RResult<Value> {
    let name = if use_sep { "paste" } else { "paste0" };
    let arity = if use_sep { 4 } else { 3 };
    let nargs = args.len();
    let correct_nargs = nargs == arity;
    if !correct_nargs && nargs + 1 != arity {
        return Err(Flow::error(format!(
            "{} argument{} passed to .Internal({}) which requires {}",
            nargs,
            if nargs == 1 { "" } else { "s" },
            name,
            arity
        )));
    }
    let x = &args[0].value;
    if !x.is_vector_list() {
        return Err(Flow::error("invalid first argument"));
    }
    let mut sep: Rc<str> = Rc::from("");
    let collapse;
    let mut recycle0 = false;
    if use_sep {
        sep = match &args[1].value {
            Value::Str(s) if !s.data.is_empty() && s.data[0].is_some() => s.data[0].clone().unwrap(),
            _ => return Err(Flow::error("invalid separator")),
        };
        collapse = args[2].value.clone();
        if correct_nargs {
            recycle0 = as_bool2(&args[3].value, call)?;
        }
    } else {
        collapse = args[1].value.clone();
        if correct_nargs {
            recycle0 = as_bool2(&args[2].value, call)?;
        }
    }
    let collapse: Option<Rc<str>> = match &collapse {
        Value::Null => None,
        Value::Str(s) if !s.data.is_empty() && s.data[0].is_some() => s.data[0].clone(),
        _ => return Err(Flow::error("invalid 'collapse' argument")),
    };
    let zero = |collapse: &Option<Rc<str>>| {
        if collapse.is_some() { Value::str("") } else { Value::strs(vec![]) }
    };
    if x.is_empty() {
        return Ok(zero(&collapse));
    }
    let parts = paste_args(interp, x, env, name)?;
    let nx = parts.len();
    let mut maxlen = 0;
    for p in &parts {
        if recycle0 && p.is_empty() {
            return Ok(zero(&collapse));
        }
        maxlen = maxlen.max(p.len());
    }
    if maxlen == 0 {
        return Ok(zero(&collapse));
    }
    let mut ans: Vec<String> = Vec::with_capacity(maxlen);
    for i in 0..maxlen {
        let mut buf = String::new();
        for (j, p) in parts.iter().enumerate() {
            if !p.is_empty() {
                buf.push_str(na_str(&p[i % p.len()]));
            }
            if j != nx - 1 && !sep.is_empty() {
                buf.push_str(&sep);
            }
        }
        ans.push(buf);
    }
    match collapse {
        Some(c) => Ok(Value::str(&ans.join(&c))),
        None => Ok(Value::strs_from(ans)),
    }
}

fn do_paste(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    paste_impl(i, c, a, e, true)
}

fn do_paste0(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    paste_impl(i, c, a, e, false)
}

// R: paste.c:do_filepath
fn do_filepath(interp: &mut Interp, _: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "file.path")?;
    let x = &args[0].value;
    if !x.is_vector_list() {
        return Err(Flow::error("invalid first argument"));
    }
    if x.is_empty() {
        return Ok(Value::strs(vec![]));
    }
    let sep = match &args[1].value {
        Value::Str(s) if !s.data.is_empty() && s.data[0].is_some() => s.data[0].clone().unwrap(),
        _ => return Err(Flow::error("invalid separator")),
    };
    // Coerce element by element, stopping at the first empty one.
    let Value::List(l) = x else { return Err(Flow::error("invalid first argument")) };
    let mut parts = Vec::new();
    for xj in &l.data {
        let one = Value::list(vec![xj.clone()]);
        let mut p = paste_args(interp, &one, env, "file.path")?;
        let p = p.pop().unwrap();
        if p.is_empty() {
            return Ok(Value::strs(vec![]));
        }
        parts.push(p);
    }
    let maxlen = parts.iter().map(|p| p.len()).max().unwrap_or(0);
    let nx = parts.len();
    let out: Vec<String> = (0..maxlen)
        .map(|i| {
            let mut buf = String::new();
            for (j, p) in parts.iter().enumerate() {
                buf.push_str(na_str(&p[i % p.len()]));
                if j != nx - 1 && !sep.is_empty() {
                    buf.push_str(&sep);
                }
            }
            buf
        })
        .collect();
    Ok(Value::strs_from(out))
}

// ----------------------------------------------------------------------
// format, format.info

const R_MIN_DIGITS_OPT: i32 = 0;
const R_MAX_DIGITS_OPT: i32 = 22;

fn with_dec(s: String, dec: &str) -> String {
    if dec == "." { s } else { s.replacen('.', dec, 1) }
}

// R: paste.c:do_format
fn do_format(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 9, "format")?;
    let mut pp = interp.print_params();
    let x = &args[0].value;
    match x {
        Value::Env(e) => return Ok(Value::str(&crate::print::encode_environment(e))),
        Value::ExtPtr(p) => {
            return Ok(Value::str(&format!("<pointer: {:p}>", Rc::as_ptr(p))));
        }
        v if v.is_atomic() => {}
        _ => return Err(Flow::error("first argument must be atomic or environment")),
    }
    let trim = as_logical(&args[1].value);
    if trim == NA_LOGICAL {
        return Err(Flow::error("invalid 'trim' argument"));
    }
    let trim = trim != 0;
    if !args[2].value.is_null() {
        let digits = as_integer(&args[2].value);
        if digits == NA_INTEGER || !(R_MIN_DIGITS_OPT..=R_MAX_DIGITS_OPT).contains(&digits) {
            return Err(Flow::error(format!("invalid value {} for 'digits' argument", digits)));
        }
        pp.digits = digits as usize;
    }
    let nsmall = as_integer(&args[3].value);
    if nsmall == NA_INTEGER || !(0..=20).contains(&nsmall) {
        return Err(Flow::error("invalid 'nsmall' argument"));
    }
    let wd = if args[4].value.is_null() { 0 } else { as_integer(&args[4].value) };
    if wd == NA_INTEGER {
        return Err(Flow::error("invalid 'width' argument"));
    }
    let wd = wd.max(0) as usize;
    let adj = as_integer(&args[5].value);
    if adj == NA_INTEGER || !(0..=3).contains(&adj) {
        return Err(Flow::error("invalid 'justify' argument"));
    }
    let na = as_logical(&args[6].value);
    if na == NA_LOGICAL {
        return Err(Flow::error("invalid 'na.encode' argument"));
    }
    let sci_arg = &args[7].value;
    if sci_arg.len() != 1 {
        return Err(Flow::error("invalid 'scientific' argument"));
    }
    let sci = match sci_arg {
        Value::Lgl(l) => {
            if l.data[0] == NA_LOGICAL {
                NA_INTEGER
            } else if l.data[0] > 0 {
                -99
            } else {
                310
            }
        }
        Value::Int(_) | Value::Real(_) => as_integer(sci_arg),
        _ => return Err(Flow::error("invalid 'scientific' argument")),
    };
    if sci != NA_INTEGER {
        pp.scipen = sci;
    }
    let dec: String = match &args[8].value {
        Value::Str(d) if d.data.len() == 1 => match &d.data[0] {
            None => match interp.get_option("OutDec") {
                Some(Value::Str(o)) if !o.data.is_empty() => o.data[0].as_deref().unwrap_or(".").to_string(),
                _ => ".".to_string(),
            },
            Some(s) => s.chars().take(10).collect(),
        },
        _ => return Err(Flow::error("invalid 'decimal.mark' argument")),
    };
    let n = x.len();
    let y: Value = if n == 0 {
        Value::strs(vec![])
    } else {
        match x {
            Value::Lgl(v) => {
                let mut w = if trim { 0 } else { format::format_logical(&v.data, pp.na_width) };
                w = w.max(wd);
                Value::strs_from(v.data.iter().map(|&b| format::encode_logical(b, w, "NA")))
            }
            Value::Int(v) => {
                let mut w = if trim { 0 } else { format::format_integer(&v.data, pp.na_width) };
                w = w.max(wd);
                Value::strs_from(v.data.iter().map(|&b| format::encode_integer(b, w, "NA")))
            }
            Value::Real(v) => {
                let (w, d, e) = format::format_real(&v.data, pp.digits, pp.scipen, nsmall as usize, pp.na_width);
                let w = if trim { 0 } else { w }.max(wd);
                Value::strs_from(v.data.iter().map(|&b| with_dec(format::encode_real0(b, w, d, e, "NA"), &dec)))
            }
            Value::Cplx(v) => {
                let (mut r, mut i) =
                    format::format_complex(&v.data, pp.digits, pp.scipen, nsmall as usize, pp.na_width);
                if trim {
                    r.0 = 0;
                    i.0 = 0;
                }
                r.0 = r.0.max(wd);
                i.0 = i.0.max(wd);
                Value::strs_from(v.data.iter().map(|&z| {
                    let s = format::encode_complex(z, r, i, "NA");
                    if dec == "." { s } else { s.replace('.', &dec) }
                }))
            }
            Value::Raw(v) => {
                // format.default sends raw vectors to as.character(); the
                // internal formats them as two hex digits.
                Value::strs_from(v.data.iter().map(|b| format!("{:02x}", b)))
            }
            Value::Str(v) => {
                let mut w = wd;
                if adj != 3 {
                    for e in &v.data {
                        match e {
                            Some(s) => w = w.max(rstrlen(s, None)),
                            None if na != 0 => w = w.max(pp.na_width),
                            None => {}
                        }
                    }
                } else {
                    w = 0;
                }
                let out: Vec<RStr> = v
                    .data
                    .iter()
                    .map(|e| {
                        if na == 0 && e.is_none() {
                            return None;
                        }
                        let s0: &str = e.as_deref().unwrap_or("NA");
                        let il = match e {
                            Some(s) => rstrlen(s, None),
                            None => pp.na_width,
                        };
                        let mut buf = String::new();
                        let mut b = w as i64 - il as i64;
                        if b > 0 && adj != 0 {
                            let b0 = if adj == 2 { b / 2 } else { b };
                            buf.push_str(&" ".repeat(b0 as usize));
                            b -= b0;
                        }
                        buf.push_str(s0);
                        if b > 0 && adj != 1 {
                            buf.push_str(&" ".repeat(b as usize));
                        }
                        Some(Rc::from(buf.as_str()))
                    })
                    .collect();
                Value::strs(out)
            }
            _ => return Err(Flow::error("Impossible mode ( x )")),
        }
    };
    let mut y = y;
    if let Some(d) = x.get_attr(s::DIM) {
        y.set_attr_raw(s::DIM, d);
        if let Some(dn) = x.get_attr(s::DIMNAMES) {
            y.set_attr_raw(s::DIMNAMES, dn);
        }
    } else if let Some(nm) = x.get_attr(s::NAMES) {
        y.set_attr_raw(s::NAMES, nm);
    }
    Ok(y)
}

// R: paste.c:do_formatinfo
fn do_formatinfo(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "format.info")?;
    let x = &args[0].value;
    let mut pp = interp.print_params();
    if !args[1].value.is_null() {
        let digits = as_integer(&args[1].value);
        if digits == NA_INTEGER || !(R_MIN_DIGITS_OPT..=R_MAX_DIGITS_OPT).contains(&digits) {
            return Err(Flow::error("invalid 'digits' argument"));
        }
        pp.digits = digits as usize;
    }
    let nsmall = as_integer(&args[2].value);
    if nsmall == NA_INTEGER || !(0..=20).contains(&nsmall) {
        return Err(Flow::error("invalid 'nsmall' argument"));
    }
    let res: Vec<i32> = match x {
        Value::Raw(_) => vec![if x.is_empty() { 0 } else { 2 }],
        Value::Lgl(v) => vec![if v.data.is_empty() { 0 } else { format::format_logical(&v.data, pp.na_width) as i32 }],
        Value::Int(v) => vec![if v.data.is_empty() { 0 } else { format::format_integer(&v.data, pp.na_width) as i32 }],
        Value::Real(v) => {
            let (w, d, e) = format::format_real(&v.data, pp.digits, pp.scipen, nsmall as usize, pp.na_width);
            vec![w as i32, d as i32, e as i32]
        }
        Value::Cplx(v) => {
            let (r, i) = format::format_complex(&v.data, pp.digits, pp.scipen, nsmall as usize, pp.na_width);
            vec![r.0 as i32, r.1 as i32, r.2 as i32, i.0 as i32, i.1 as i32, i.2 as i32]
        }
        Value::Str(v) => {
            let w = v.data.iter().filter_map(|e| e.as_ref().map(|s| rstrlen(s, None))).max().unwrap_or(0);
            vec![w as i32]
        }
        _ => return Err(Flow::error("atomic vector arguments only")),
    };
    Ok(Value::ints(res))
}

// ----------------------------------------------------------------------
// formatC

/// R: nmath/fround.c:R_pow_di
fn r_pow_di(mut x: f64, mut n: i32) -> f64 {
    let mut xn = 1.0;
    if x.is_nan() {
        return x;
    }
    if n != 0 {
        if !x.is_finite() {
            return x.powf(n as f64);
        }
        let is_neg = n < 0;
        if is_neg {
            n = -n;
        }
        loop {
            if n & 1 != 0 {
                xn *= x;
            }
            n >>= 1;
            if n != 0 {
                x *= x;
            } else {
                break;
            }
        }
        if is_neg {
            xn = 1.0 / xn;
        }
    }
    xn
}

/// R: nmath/fround.c:fround
fn fround(x: f64, digits: f64) -> f64 {
    const MAX_DIGITS: f64 = 308.0 + 15.0;
    let max10e = 308i32;
    if x.is_nan() || digits.is_nan() {
        return x + digits;
    }
    if !x.is_finite() {
        return x;
    }
    if digits > MAX_DIGITS || x == 0.0 {
        return x;
    } else if digits < -(max10e as f64) {
        return 0.0;
    } else if digits == 0.0 {
        return x.round_ties_even();
    }
    let dig = (digits + 0.5).floor() as i32;
    let (sgn, x) = if x < 0.0 { (-1.0, -x) } else { (1.0, x) };
    let l10x = std::f64::consts::LOG10_2 * (0.5 + logb(x));
    if l10x + dig as f64 > 15.0 {
        return sgn * x;
    }
    let (i10, xd, xu);
    if dig <= max10e {
        let pow10 = r_pow_di(10.0, dig);
        let x10 = x * pow10;
        i10 = x10.floor();
        xd = i10 / pow10;
        xu = x10.ceil() / pow10;
    } else {
        let e10 = dig - max10e;
        let p10 = r_pow_di(10.0, e10);
        let pow10 = r_pow_di(10.0, max10e);
        let x10 = (x * pow10) * p10;
        i10 = x10.floor();
        xd = i10 / pow10 / p10;
        xu = x10.ceil() / pow10 / p10;
    }
    let du = xu - x;
    let dd = x - xd;
    sgn * (if du < dd || (i10 % 2.0 == 1.0 && du == dd) { xu } else { xd })
}

/// C `logb` for finite positive `x`: the binary exponent.
fn logb(x: f64) -> f64 {
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i64;
    if e == 0 {
        // subnormal: position of the leading mantissa bit
        let m = bits & ((1u64 << 52) - 1);
        (-1074 + 63 - m.leading_zeros() as i64) as f64
    } else {
        (e - 1023) as f64
    }
}

/// Rexp10
fn rexp10(x: f64) -> f64 {
    10f64.powf(x)
}

/// snprintf into a buffer of `cap` bytes: at most `cap - 1` characters.
fn bounded(s: String, cap: usize) -> String {
    if s.len() < cap {
        s
    } else {
        let mut end = cap.saturating_sub(1);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
    }
}

/// Substitute `*` in a C format with the given numbers (R passes width and
/// precision as `*` arguments to snprintf).
fn star(form: &str, vals: &[i32]) -> String {
    let mut out = String::with_capacity(form.len() + 8);
    let mut k = 0;
    for c in form.chars() {
        if c == '*' && k < vals.len() {
            out.push_str(&vals[k].to_string());
            k += 1;
        } else {
            out.push(c);
        }
    }
    out
}

// R: util.c:str_signif
#[allow(clippy::too_many_arguments)]
fn str_signif(
    x: &Value,
    ty: &str,
    width: i32,
    digits: i32,
    fmt: &str,
    flag: &str,
    caps: &[usize],
) -> RResult<Vec<String>> {
    let dig = digits.abs();
    let do_fg = fmt == "fg";
    if width == 0 {
        return Err(Flow::error("width cannot be zero"));
    }
    let n = x.len();
    let mut out = Vec::with_capacity(n);
    if fmt == "d" {
        let form = if flag.is_empty() { "%*d".to_string() } else { format!("%{}*d", flag) };
        if ty == "integer" {
            let Value::Int(v) = x else { return Err(Flow::error("unsupported type ")) };
            for (i, &xi) in v.data.iter().enumerate() {
                let f = star(&form, &[width]);
                out.push(bounded(c_format(&f, CArg::Int(xi)), caps[i] + 1));
            }
        } else {
            return Err(Flow::error("'type' must be \"integer\" for  \"d\"-format"));
        }
        return Ok(out);
    }
    let mut form = if flag.is_empty() { "%*.*".to_string() } else { format!("%{}*.*", flag) };
    let f0 = format!("%{}.*f", flag);
    if do_fg {
        form.push('g');
    } else {
        form.push_str(fmt);
    }
    if ty != "double" {
        return Err(Flow::error("'type' must be \"real\" for this format"));
    }
    let xs: Vec<f64> = match x {
        Value::Real(v) => v.data.clone(),
        Value::Int(v) => v.data.iter().map(|&i| crate::coerce::int_to_real(i)).collect(),
        _ => return Err(Flow::error("unsupported type ")),
    };
    for (i, &xx0) in xs.iter().enumerate() {
        let cap = caps[i] + 1;
        if do_fg {
            let mut xx = xx0;
            if xx == 0.0 {
                out.push("0".to_string());
                continue;
            }
            let xxx = xx.abs();
            let mut iex = (xxx.log10() + 1e-12).floor() as i32;
            let x_ = fround(xxx / rexp10(iex as f64) + 1e-12, (dig - 1) as f64);
            if iex > 0 && x_ >= 10.0 {
                xx = x_ * rexp10(iex as f64);
                iex += 1;
            }
            if iex == -4 && xx.abs() < 1e-4 {
                iex = -5;
            }
            if iex < -4 {
                let f = star(&f0, &[dig - 1 - iex]);
                let mut r = bounded(c_format(&f, CArg::Double(xx)), cap);
                if digits >= 0 {
                    while r.ends_with('0') {
                        r.pop();
                    }
                }
                out.push(r);
            } else {
                let p = if iex >= dig { iex + 1 } else { dig };
                let f = star(&form, &[width, p]);
                out.push(bounded(c_format(&f, CArg::Double(xx)), cap));
            }
        } else {
            let f = star(&form, &[width, dig]);
            out.push(bounded(c_format(&f, CArg::Double(xx0)), cap));
        }
    }
    Ok(out)
}

// R: util.c:do_formatC
fn do_formatc(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 7, "formatC")?;
    let x = &args[0].value;
    if !x.is_vector() {
        return Err(Flow::error("'x' must be a vector"));
    }
    let first_str = |v: &Value| -> String {
        match v {
            Value::Str(s) if !s.data.is_empty() => s.data[0].as_deref().unwrap_or("NA").to_string(),
            _ => String::new(),
        }
    };
    let ty = first_str(&args[1].value);
    let width = as_integer(&args[2].value);
    let digits = as_integer(&args[3].value);
    let fmt = first_str(&args[4].value);
    let flag = first_str(&args[5].value);
    let istrlen = interp.coerce(&args[6].value, VType::Int)?;
    let caps: Vec<usize> = match &istrlen {
        Value::Int(v) => v.data.iter().map(|&k| (k.max(0) + 2) as usize).collect(),
        _ => vec![],
    };
    if caps.len() < x.len() {
        return Err(Flow::error("invalid 'i.strlen' argument"));
    }
    let r = str_signif(x, &ty, width, digits, &fmt, &flag, &caps)?;
    Ok(Value::strs_from(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fround_basic() {
        assert_eq!(fround(2.5, 0.0), 2.0);
        assert_eq!(fround(1.2345, 2.0), 1.23);
        assert_eq!(fround(0.15, 1.0), 0.1);
    }
}
