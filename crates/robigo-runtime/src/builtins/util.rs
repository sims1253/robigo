//! Miscellaneous internals from GNU R's util.c.

use super::internal;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static DEFS: &[Builtin] =
    &[internal("compareNumericVersion", do_compare_numeric_version, 2), internal("crc64", do_crc64, 1)];

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
