//! Serialization builtins: `serialize`, `unserialize`, `lazyLoadDBfetch`
//! (GNU R's serialize.c).

use std::io::Read;

use super::{arg, internal};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("serialize", do_serialize, 5),
    internal("serializeb", do_serialize, 5),
    internal("unserialize", do_unserialize, 2),
    internal("lazyLoadDBfetch", do_lazy_load_db_fetch, 4),
];

fn do_serialize(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    if !arg(&args, 1).is_null() {
        return Err(Flow::error("rr: serialize() to connections is not implemented yet"));
    }
    Ok(Value::raws(crate::serialize::serialize(&x)))
}

fn do_unserialize(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let hook = arg(&args, 1);
    match arg(&args, 0) {
        Value::Raw(r) => crate::serialize::unserialize(i, &r.data, hook),
        Value::Str(s) => {
            let text = s.data.first().cloned().flatten().unwrap_or_default();
            crate::serialize::unserialize(i, text.as_bytes(), hook)
        }
        _ => Err(Flow::error("rr: unserialize() from connections is not implemented yet")),
    }
}

/// Decompress a lazy-load database entry (R_decompress1/3).
pub fn decompress(data: &[u8], method: i32) -> RResult<Vec<u8>> {
    let corrupt = || Flow::error("lazy-load database is corrupt");
    match method {
        0 => Ok(data.to_vec()),
        1 => {
            if data.len() < 4 {
                return Err(corrupt());
            }
            let outlen = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
            let mut out = Vec::with_capacity(outlen);
            flate2::read::ZlibDecoder::new(&data[4..]).read_to_end(&mut out).map_err(|_| corrupt())?;
            Ok(out)
        }
        3 => {
            if data.len() < 5 {
                return Err(corrupt());
            }
            let outlen = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
            match data[4] {
                b'Z' => {
                    let mut out = Vec::with_capacity(outlen);
                    flate2::read::ZlibDecoder::new(&data[5..]).read_to_end(&mut out).map_err(|_| corrupt())?;
                    Ok(out)
                }
                b'0' => Ok(data[5..].to_vec()),
                b'2' => Err(Flow::error("rr: bzip2-compressed lazy-load databases are not supported yet")),
                b'X' => Err(Flow::error("rr: xz-compressed lazy-load databases are not supported yet")),
                _ => Err(corrupt()),
            }
        }
        2 => Err(Flow::error("rr: bzip2-compressed lazy-load databases are not supported yet")),
        _ => Err(corrupt()),
    }
}

fn do_lazy_load_db_fetch(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let key = crate::coerce::to_int(&arg(&args, 0), &mut Default::default());
    let file = crate::coerce::as_string(&arg(&args, 1)).ok_or_else(|| Flow::error("bad file name"))?;
    let compressed = crate::coerce::as_integer(&arg(&args, 2));
    let hook = arg(&args, 3);
    if key.len() != 2 {
        return Err(Flow::error("bad offset/length argument"));
    }
    let (off, len) = (key[0] as usize, key[1] as usize);
    let mut f = std::fs::File::open(&*file).map_err(|_| Flow::error(format!("cannot open file '{}': No such file or directory", file)))?;
    use std::io::Seek;
    f.seek(std::io::SeekFrom::Start(off as u64)).map_err(|_| Flow::error("seek failed"))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).map_err(|_| Flow::error("read failed"))?;
    let data = decompress(&buf, if compressed == NA_INTEGER { 0 } else { compressed })?;
    let v = crate::serialize::unserialize(i, &data, hook)?;
    match v {
        Value::Promise(p) => i.force_promise(&p),
        v => Ok(v),
    }
}
