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
    internal("serializeToConn", do_serialize_to_conn, 5).invisible(),
    internal("unserializeFromConn", do_unserialize_from_conn, 2),
    super::prim("lazyLoadDBfetch", do_lazy_load_db_fetch, 4),
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

/// `saveRDS` and `serialize(x, con)`.
// R: serialize.c:do_serializeToConn
fn do_serialize_to_conn(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    use super::io;
    let object = arg(&args, 0);
    let con = io::connection_number(&arg(&args, 1))?;
    let Value::Lgl(a) = arg(&args, 2) else { return Err(Flow::error("'ascii' must be logical")) };
    let ascii = a.data.first().copied().unwrap_or(NA_LOGICAL);
    let version = match arg(&args, 3) {
        Value::Null => 3,
        v => crate::coerce::as_integer(&v),
    };
    if version == NA_INTEGER || version <= 0 {
        return Err(Flow::error("bad version value"));
    }
    if version < 2 {
        return Err(Flow::error(format!("cannot save to connections in version {} format", version)));
    }
    if !arg(&args, 4).is_null() {
        return Err(Flow::error("rr: serialization hooks are not implemented yet"));
    }
    if ascii != 0 || version != 3 {
        return Err(Flow::error("rr: only binary (XDR) serialization version 3 is implemented"));
    }
    let (wasopen, ..) = io::conn_state(i, con)?;
    if !wasopen && !io::open_in_mode(i, con, "wb")? {
        return Err(Flow::error("cannot open the connection"));
    }
    let (_, _, canwrite, text) = io::conn_state(i, con)?;
    let r = (|| {
        if text {
            return Err(Flow::error("binary-mode connection required for ascii=FALSE"));
        }
        if !canwrite {
            return Err(Flow::error("connection not open for writing"));
        }
        io::con_write_bytes(i, con, &crate::serialize::serialize(&object))
    })();
    if !wasopen {
        io::close_conn(i, con)?;
    }
    r.map(|_| Value::Null)
}

/// `readRDS` and `unserialize(con)`.
// R: serialize.c:do_unserializeFromConn
fn do_unserialize_from_conn(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    use super::io;
    let con = io::connection_number(&arg(&args, 0))?;
    let (wasopen, ..) = io::conn_state(i, con)?;
    if !wasopen && !io::open_in_mode(i, con, "rb")? {
        return Err(Flow::error("cannot open the connection"));
    }
    let r = (|| {
        let (_, canread, ..) = io::conn_state(i, con)?;
        if !canread {
            return Err(Flow::error("connection not open for reading"));
        }
        let bytes = io::con_read_all(i, con)?;
        crate::serialize::unserialize(i, &bytes, arg(&args, 1))
    })();
    if !wasopen {
        io::close_conn(i, con)?;
    }
    r
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
