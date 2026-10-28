//! Miscellaneous internals from GNU R's util.c.

use super::internal;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("compareNumericVersion", do_compare_numeric_version, 2),
    internal("crc64", do_crc64, 1),
    internal("pretty", do_pretty, 8),
];

/// CRC-64/XZ as liblzma's `lzma_crc64` computes it (poly 0xC96C5795D7870F42,
/// reflected, init and final xor of all ones).
// R: xz src/liblzma/check/crc64_table.c + crc64_fast.c:lzma_crc64
fn lzma_crc64(bytes: &[u8]) -> u64 {
    const POLY: u64 = 0xC96C_5795_D787_0F42;
    static TABLE: std::sync::OnceLock<[u64; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u64; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u64;
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
            }
            *e = c;
        }
        t
    });
    let mut crc = !0u64;
    for &b in bytes {
        crc = table[((crc ^ b as u64) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

// R: util.c:do_crc64
fn crc64(input: &Value) -> RResult<Value> {
    let Value::Str(s) = input else { return Err(Flow::error("input must be a character string")) };
    // R reads element 0 only, and CHAR(NA_STRING) is "NA".
    let text = s.data.first().map(|e| e.as_deref().unwrap_or("NA")).unwrap_or("");
    Ok(Value::strs_from([format!("{:x}", lzma_crc64(text.as_bytes()))]))
}

fn do_crc64(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if args.len() != 1 {
        // R: util.c:checkArity -> .Internal uses error() (the context call).
        return Err(Flow::error(format!(
            "{} argument{} passed to .Internal(crc64) which requires 1",
            args.len(),
            if args.len() == 1 { "" } else { "s" }
        )));
    }
    crc64(&args[0].value)
}

/// The `.Call` entry point of the same routine (utils' `crc64`).
// R: src/library/utils/src/utils.c:crc64
pub fn native_crc64(args: &[Arg]) -> RResult<Value> {
    match args.first() {
        Some(a) => crc64(&a.value),
        None => Err(Flow::error("input must be a character string")),
    }
}

// R: util.c:compareNumericVersion
fn compare_numeric_version(x: &Value, y: &Value) -> RResult<i32> {
    let Value::Int(x) = x else { return Err(Flow::error("invalid 'x' argument")) };
    let Value::Int(y) = y else { return Err(Flow::error("invalid 'y' argument")) };
    let (ix, iy) = (&x.data, &y.data);
    let nc = ix.len().min(iy.len());
    if nc == 0 {
        return Ok(NA_INTEGER);
    }
    for i in 0..nc {
        if ix[i] > iy[i] {
            return Ok(1);
        }
        if ix[i] < iy[i] {
            return Ok(-1);
        }
    }
    if ix[nc..].iter().any(|&v| v > 0) {
        return Ok(1);
    }
    if iy[nc..].iter().any(|&v| v > 0) {
        return Ok(-1);
    }
    Ok(0)
}

// R: util.c:do_compareNumericVersion
fn do_compare_numeric_version(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let elts = |v: &Value, what: &str| -> RResult<Vec<Value>> {
        match v {
            Value::List(l) => Ok(l.data.clone()),
            v if v.is_vector() => Ok(crate::coerce::to_list(v)),
            _ => Err(Flow::error(format!("invalid '{}' argument", what))),
        }
    };
    let x = elts(&args[0].value, "x")?;
    let y = elts(&args[1].value, "y")?;
    let na = if !x.is_empty() && !y.is_empty() { x.len().max(y.len()) } else { 0 };
    let mut out = Vec::with_capacity(na);
    for i in 0..na {
        out.push(compare_numeric_version(&x[i % x.len()], &y[i % y.len()])?);
    }
    Ok(Value::ints(out))
}

// ----------------------------------------------------------------------
// pretty()

/// `R_pretty`: the "pretty" interval algorithm.
// R: appl/pretty.c:R_pretty
fn r_pretty(
    lo: &mut f64,
    up: &mut f64,
    ndiv: &mut i32,
    min_n: i32,
    shrink_sml: f64,
    high_u_fact: [f64; 3],
    eps_correction: i32,
    return_bounds: bool,
) -> f64 {
    const ROUNDING_EPS: f64 = 1e-10;
    let h = high_u_fact[0];
    let h5 = high_u_fact[1];
    let f_min = high_u_fact[2];

    let (lo_, up_) = (*lo, *up);
    let dx = up_ - lo_;
    let cell;
    let i_small;
    if dx == 0.0 && up_ == 0.0 {
        cell = 1.0;
        i_small = true;
    } else {
        let mut cell0 = lo_.abs().max(up_.abs());
        let mut u = 1.0 + if h5 >= 1.5 * h + 0.5 { 1.0 / (1.0 + h) } else { 1.5 / (1.0 + h5) };
        u *= 1.max(*ndiv) as f64 * f64::EPSILON;
        i_small = dx < cell0 * u * 3.0;
        if i_small {
            if cell0 > 10.0 {
                cell0 = 9.0 + cell0 / 10.0;
            }
            cell0 *= shrink_sml;
            if min_n > 1 {
                cell0 /= min_n as f64;
            }
        } else {
            cell0 = dx;
            if dx.is_finite() {
                if *ndiv > 1 {
                    cell0 /= *ndiv as f64;
                }
            } else if *ndiv >= 2 {
                cell0 = up_ / (*ndiv as f64) - lo_ / (*ndiv as f64);
            }
        }
        cell = cell0;
    }

    let max_f = 1.25;
    let mut cell = cell;
    let subsmall = {
        let s = f_min * f64::MIN_POSITIVE;
        if s == 0.0 { f64::MIN_POSITIVE } else { s }
    };
    if cell < subsmall {
        cell = subsmall;
    } else if cell > f64::MAX / max_f {
        cell = f64::MAX / max_f;
    }

    // NB: R relies on an exact pow(10, floor(log10(cell))).
    let base = 10.0f64.powf(cell.log10().floor());
    let mut unit = base;
    let u2 = 2.0 * base;
    if u2 - cell < h * (cell - unit) {
        unit = u2;
        let u5 = 5.0 * base;
        if u5 - cell < h5 * (cell - unit) {
            unit = u5;
            let u10 = 10.0 * base;
            if u10 - cell < h * (cell - unit) {
                unit = u10;
            }
        }
    }

    let mut ns = (lo_ / unit + ROUNDING_EPS).floor();
    let mut nu = (up_ / unit - ROUNDING_EPS).ceil();
    if eps_correction != 0 && (eps_correction > 1 || !i_small) {
        if lo_ != 0.0 {
            *lo *= 1.0 - f64::EPSILON;
        } else {
            *lo = -f64::MIN_POSITIVE;
        }
        if up_ != 0.0 {
            *up *= 1.0 + f64::EPSILON;
        } else {
            *up = f64::MIN_POSITIVE;
        }
    }
    while ns * unit > *lo + ROUNDING_EPS * unit {
        ns -= 1.0;
    }
    while !(ns * unit).is_finite() {
        ns += 1.0;
    }
    while nu * unit < *up - ROUNDING_EPS * unit {
        nu += 1.0;
    }
    while !(nu * unit).is_finite() {
        nu -= 1.0;
    }

    let mut k = (0.5 + nu - ns) as i32;
    if k < min_n {
        k = min_n - k;
        if lo_ == 0.0 && ns == 0.0 && up_ != 0.0 {
            nu += k as f64;
        } else if up_ == 0.0 && nu == 0.0 && lo_ != 0.0 {
            ns -= k as f64;
        } else if ns >= 0.0 {
            nu += (k / 2) as f64;
            ns -= (k / 2 + k % 2) as f64;
        } else {
            ns -= (k / 2) as f64;
            nu += (k / 2 + k % 2) as f64;
        }
        *ndiv = min_n;
    } else {
        *ndiv = k;
    }
    if return_bounds {
        if ns * unit < *lo {
            *lo = ns * unit;
        }
        if nu * unit > *up {
            *up = nu * unit;
        }
    } else {
        *lo = ns;
        *up = nu;
    }
    unit
}

// R: util.c:do_pretty
fn do_pretty(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    super::character::check_arity_internal(&args, 8, "pretty")?;
    let mut lo = crate::coerce::as_real(&args[0].value);
    if !lo.is_finite() {
        return Err(Flow::error("invalid 'l' argument"));
    }
    let mut up = crate::coerce::as_real(&args[1].value);
    if !up.is_finite() {
        return Err(Flow::error("invalid 'u' argument"));
    }
    let mut n = crate::coerce::as_integer(&args[2].value);
    if n == NA_INTEGER || n < 0 {
        return Err(Flow::error("invalid 'n' argument"));
    }
    let min_n = crate::coerce::as_integer(&args[3].value);
    if min_n == NA_INTEGER || min_n < 0 || min_n > n {
        return Err(Flow::error("invalid 'min.n' argument"));
    }
    let shrink = crate::coerce::as_real(&args[4].value);
    if !shrink.is_finite() || shrink <= 0.0 {
        return Err(Flow::error("invalid 'shrink.sml' argument"));
    }
    // coerceVector(CAR(args), REALSXP): (high.u.bias, u5.bias, f.min)
    let hi = crate::coerce::to_real(&args[5].value, &mut crate::coerce::CoerceWarnings::default());
    let z = |k: usize| hi.get(k).copied().unwrap_or(f64::NAN);
    if !z(0).is_finite() || z(0) < 0.0 {
        return Err(Flow::error("invalid 'high.u.bias' argument"));
    }
    if !z(1).is_finite() || z(1) < 0.0 {
        return Err(Flow::error("invalid 'u5.bias' argument"));
    }
    if !z(2).is_finite() || z(2) <= 0.0 {
        return Err(Flow::error("invalid 'f.min' argument"));
    }
    let eps = crate::coerce::as_integer(&args[6].value);
    if eps == NA_INTEGER || !(0..=2).contains(&eps) {
        return Err(Flow::error("'eps.correct' must be 0, 1, or 2"));
    }
    let bounds = crate::coerce::as_logical(&args[7].value);
    if bounds == NA_LOGICAL {
        return Err(Flow::error("'bounds' must be TRUE or FALSE"));
    }
    let return_bounds = bounds != 0;
    let hf = [z(0), z(1), z(2)];
    let unit = r_pretty(&mut lo, &mut up, &mut n, min_n, shrink, hf, eps, return_bounds);
    let (names, values) = if return_bounds {
        (vec!["l", "u", "n"], vec![Value::real(lo), Value::real(up), Value::int(n)])
    } else {
        (vec!["ns", "nu", "n", "unit"], vec![Value::real(lo), Value::real(up), Value::int(n), Value::real(unit)])
    };
    let mut ans = Value::list(values);
    ans.set_attr_raw(crate::sym::s::NAMES, Value::strs_from(names));
    Ok(ans)
}
