//! Serialization builtins: `serialize`, `unserialize`, `lazyLoadDBfetch`
//! (GNU R's serialize.c).

use std::io::Read;

use super::{arg, internal};
use crate::coerce::{as_integer, as_logical};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("serialize", do_serialize, 5),
    internal("serializeb", do_serializeb, 5),
    internal("unserialize", do_unserialize, 2),
    internal("serializeToConn", do_serialize_to_conn, 5).invisible(),
    internal("unserializeFromConn", do_unserialize_from_conn, 2),
    internal("serializeInfoFromConn", do_serialize_info_from_conn, 1),
    super::prim("lazyLoadDBfetch", do_lazy_load_db_fetch, 4),
];

/// R: serialize.c R_serialize — `icon == NULL` returns the bytes as a raw
/// vector, otherwise they go to a connection. `type`: 0 xdr (default),
/// 1 ascii, 2 asciihex, 3 binary.
fn do_serialize(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let object = arg(&args, 0);
    let format = as_integer(&arg(&args, 2));
    let version = serialize_version_arg(&arg(&args, 3))?;
    let bytes = match format {
        3 => crate::serialize::serialize_binary(&object, version),
        1 | 2 => return Err(Flow::error("robigo: ascii serialization is not implemented yet")),
        _ => crate::serialize::serialize_version(&object, version),
    };
    match arg(&args, 1) {
        Value::Null => Ok(Value::raws(bytes)),
        con => {
            use super::io;
            let n = io::connection_number(&con)?;
            write_to_conn(i, n, &bytes, format)?;
            Ok(Value::Null)
        }
    }
}

/// R: serialize.c R_serializeb — to a connection, xdr if `xdr` else native
/// binary. Used for socket connections.
fn do_serializeb(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let object = arg(&args, 0);
    use super::io;
    let n = io::connection_number(&arg(&args, 1))?;
    let xdr = as_logical(&arg(&args, 2));
    let format = if xdr == 0 { 3 } else { 0 };
    let version = serialize_version_arg(&arg(&args, 3))?;
    let bytes = match format {
        3 => crate::serialize::serialize_binary(&object, version),
        _ => crate::serialize::serialize_version(&object, version),
    };
    write_to_conn(i, n, &bytes, format)
}

/// The `version` argument shared by all entry points (R: `bad version value`).
fn serialize_version_arg(v: &Value) -> RResult<i32> {
    let version = match v {
        Value::Null => crate::serialize::default_serialize_version(),
        v => as_integer(v),
    };
    if version == NA_INTEGER || version <= 0 {
        return Err(Flow::error("bad version value"));
    }
    Ok(version)
}

/// Write serialized bytes to a connection the way `R_InitConnOutPStream`
/// does: open it if needed, check mode and write.
fn write_to_conn(i: &mut Interp, n: i32, bytes: &[u8], format: i32) -> RResult<Value> {
    use super::io;
    let (wasopen, ..) = io::conn_state(i, n)?;
    if !wasopen && !io::open_in_mode(i, n, if format == 1 || format == 2 { "w" } else { "wb" })? {
        return Err(Flow::error("cannot open the connection"));
    }
    let r = (|| {
        let (_, _, canwrite, text) = io::conn_state(i, n)?;
        if format != 1 && format != 2 && text {
            return Err(Flow::error("binary-mode connection required for ascii=false"));
        }
        if !canwrite {
            return Err(Flow::error("connection not open for writing"));
        }
        io::con_write_bytes(i, n, bytes).map(|_| Value::Null)
    })();
    if !wasopen {
        io::close_conn(i, n)?;
    }
    r
}

fn do_unserialize(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let hook = arg(&args, 1);
    match arg(&args, 0) {
        Value::Raw(r) => crate::serialize::unserialize(i, &r.data, hook),
        Value::Str(s) => {
            let text = s.data.first().cloned().flatten().unwrap_or_default();
            crate::serialize::unserialize(i, text.as_bytes(), hook)
        }
        _ => Err(Flow::error("robigo: unserialize() from connections is not implemented yet")),
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
        return Err(Flow::error("robigo: serialization hooks are not implemented yet"));
    }
    if ascii != 0 {
        return Err(Flow::error("robigo: ascii serialization is not implemented yet"));
    }
    let bytes = crate::serialize::serialize_version(&object, version);
    write_to_conn(i, con, &bytes, 0)
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

/// `infoRDS`: the header of a serialized stream.
// R: serialize.c R_SerializeInfo
fn do_serialize_info_from_conn(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
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
        crate::serialize::serialize_info(&bytes)
    })();
    if !wasopen {
        io::close_conn(i, con)?;
    }
    r
}

/// Why a lazy-load entry could not be decompressed.
pub enum DecompressFail {
    /// R reports the database as corrupt.
    Corrupt,
    /// robigo cannot handle this compression yet.
    Unsupported(&'static str),
}

/// Decompress a lazy-load database entry (R: connections.c R_decompress1/3).
pub fn decompress(data: &[u8], method: i32) -> Result<Vec<u8>, DecompressFail> {
    match method {
        3 => decompress3(data),
        2 => Err(DecompressFail::Unsupported("bzip2")),
        1 => decompress1(data),
        0 | _ => Ok(data.to_vec()),
    }
}

/// R: connections.c R_decompress1 — 4-byte big-endian output length, then a
/// zlib stream.
fn decompress1(data: &[u8]) -> Result<Vec<u8>, DecompressFail> {
    if data.len() < 4 {
        return Err(DecompressFail::Corrupt);
    }
    let outlen = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    zlib_stream(&data[4..], outlen)
}

fn zlib_stream(data: &[u8], outlen: usize) -> Result<Vec<u8>, DecompressFail> {
    let mut out = Vec::with_capacity(outlen);
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).map_err(|_| DecompressFail::Corrupt)?;
    Ok(out)
}

/// R: connections.c R_decompress3 — 4-byte big-endian output length, a type
/// byte and the stream. Type 'Z' is a raw LZMA2 stream (liblzma, preset 6),
/// '1' zlib, '2' bzip2, '0' uncompressed.
fn decompress3(data: &[u8]) -> Result<Vec<u8>, DecompressFail> {
    if data.len() < 5 {
        return Err(DecompressFail::Corrupt);
    }
    let outlen = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    match data[4] {
        b'Z' => {
            use liblzma::stream::{Action, Filters, LzmaOptions, Status, Stream};
            let opts = LzmaOptions::new_preset(6).map_err(|_| DecompressFail::Corrupt)?;
            let mut filters = Filters::new();
            filters.lzma2(&opts);
            let mut strm = Stream::new_raw_decoder(&filters).map_err(|_| DecompressFail::Corrupt)?;
            let mut out = vec![0u8; outlen];
            match strm.process(&data[5..], &mut out, Action::Finish) {
                Ok(Status::StreamEnd | Status::Ok) => Ok(out),
                _ => Err(DecompressFail::Corrupt),
            }
        }
        b'1' => zlib_stream(&data[5..], outlen),
        b'2' => Err(DecompressFail::Unsupported("bzip2")),
        b'0' => Ok(data[5..].to_vec()),
        _ => Err(DecompressFail::Corrupt),
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
    let mut f = std::fs::File::open(&*file)
        .map_err(|_| Flow::error(format!("cannot open file '{}': No such file or directory", file)))?;
    use std::io::Seek;
    f.seek(std::io::SeekFrom::Start(off as u64)).map_err(|_| Flow::error("seek failed"))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).map_err(|_| Flow::error("read failed"))?;
    // R: serialize.c do_lazyLoadDBfetch — asInteger(compressed) decides the
    // method; NA is truthy in C and selects R_decompress1.
    let method = if compressed == NA_INTEGER { 1 } else { compressed };
    let data = decompress(&buf, method).map_err(|e| match e {
        DecompressFail::Corrupt => Flow::error(format!("lazy-load database '{}' is corrupt", file)),
        DecompressFail::Unsupported(what) => {
            Flow::error(format!("robigo: {} lazy-load databases are not implemented yet", what))
        }
    })?;
    // R: serialize.c do_lazyLoadDBfetch — eval(val, R_GlobalEnv)
    let v = crate::serialize::unserialize(i, &data, hook)?;
    match v {
        Value::Promise(p) => {
            let expr = p.expr.clone();
            let g = i.global.clone();
            i.eval(&expr, &g)
        }
        v => Ok(v),
    }
}
