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
    internal("saveToConn", do_save_to_conn, 6).invisible(),
    internal("loadFromConn2", do_load_from_conn2, 3).invisible(),
    internal("loadInfoFromConn2", do_load_info_from_conn2, 1),
    super::prim("lazyLoadDBfetch", do_lazy_load_db_fetch, 4),
    internal("getVarsFromFrame", do_get_vars_from_frame, 3),
    internal("lazyLoadDBinsertValue", do_lazy_load_db_insert_value, 5),
    internal("lazyLoadDBflush", do_lazy_load_db_flush, 1),
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

/// `save(list, con, ascii, version, envir, eval.promises)` — `save()` after
/// its R wrapper has resolved the connection.
// R: saveload.c do_saveToConn
fn do_save_to_conn(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    use super::io;
    let list = match arg(&args, 0) {
        Value::Str(s) => s.data.clone(),
        _ => return Err(Flow::error("first argument must be a character vector")),
    };
    let con = io::connection_number(&arg(&args, 1))?;
    let ascii = crate::coerce::as_logical(&arg(&args, 2));
    let version = match arg(&args, 3) {
        Value::Null => crate::serialize::default_serialize_version(),
        v => as_integer(&v),
    };
    if version == NA_INTEGER || version <= 0 {
        return Err(Flow::error("invalid 'version' argument"));
    }
    if version < 2 {
        return Err(Flow::error(format!("cannot save to connections in version {} format", version)));
    }
    let source = arg(&args, 4);
    let env = match &source {
        Value::Env(e) => e.clone(),
        Value::Null => i.global.clone(),
        _ => return Err(Flow::error("invalid 'environment' argument")),
    };
    let ep = crate::coerce::as_logical(&arg(&args, 5));
    if ep == NA_LOGICAL {
        return Err(Flow::error("invalid 'eval.promises' argument"));
    }
    if ascii != 0 {
        return Err(Flow::error("robigo: ascii serialization is not implemented yet"));
    }
    let (wasopen, ..) = io::conn_state(i, con)?;
    if !wasopen && !io::open_in_mode(i, con, "wb")? {
        return Err(Flow::error("cannot open the connection"));
    }
    let r = (|| -> RResult<()> {
        if !io::conn_state(i, con)?.2 {
            return Err(Flow::error("connection not open for writing"));
        }
        if io::conn_state(i, con)?.3 {
            return Err(Flow::error("cannot save XDR format to a text-mode connection"));
        }
        // R writes the magic number, then a tagged pairlist of the objects.
        let magic = format!("RDX{}\n", version);
        io::con_write_bytes(i, con, magic.as_bytes())?;
        let mut items: Vec<Arg> = Vec::with_capacity(list.len());
        for name in &list {
            let Some(name) = name.as_deref() else { continue };
            let sym = crate::sym::Sym::new(name);
            let val =
                env.lookup(sym).map(|(v, _)| v).ok_or_else(|| Flow::error(format!("object '{}' not found", name)))?;
            items.push(Arg::new(Some(sym), val));
        }
        let bytes = crate::serialize::serialize_version(&Value::pairlist(items), version);
        io::con_write_bytes(i, con, &bytes)
    })();
    if !wasopen {
        io::close_conn(i, con)?;
    }
    r.map(|_| Value::Null)
}

/// `loadFromConn2(con, envir, verbose)`.
// R: saveload.c do_loadFromConn2
fn do_load_from_conn2(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    use super::io;
    let con = io::connection_number(&arg(&args, 0))?;
    let (wasopen, ..) = io::conn_state(i, con)?;
    if !wasopen && !io::open_in_mode(i, con, "rb")? {
        return Err(Flow::error("cannot open the connection"));
    }
    let r = (|| -> RResult<Value> {
        if !io::conn_state(i, con)?.1 {
            return Err(Flow::error("connection not open for reading"));
        }
        if io::conn_state(i, con)?.3 {
            return Err(Flow::error("can only load() from a binary connection"));
        }
        let venv = arg(&args, 1);
        let Value::Env(aenv) = &venv else {
            if venv.is_null() {
                return Err(Flow::error("use of NULL environment is defunct"));
            }
            return Err(Flow::error("invalid 'envir' argument"));
        };
        let mut magic = [0u8; 5];
        let count = io::con_read_bytes(i, con, &mut magic)?;
        if count == 0 {
            return Err(Flow::error("no input is available"));
        }
        let ok = ["RDA2\n", "RDB2\n", "RDX2\n", "RDA3\n", "RDB3\n", "RDX3\n"]
            .iter()
            .any(|m| magic.starts_with(m.as_bytes()));
        if !ok {
            return Err(Flow::error(
                "the input does not start with a magic number compatible with loading from a connection",
            ));
        }
        // The rest of the stream is an ordinary serialized object.
        let rest = io::con_read_all(i, con)?;
        let obj = crate::serialize::unserialize(i, &rest, Value::Null)?;
        // R: saveload.c RestoreToEnv — a pairlist (or named list) of objects.
        let mut names: Vec<RStr> = Vec::new();
        match &obj {
            Value::Pairlist(p) => {
                for a in &p.items {
                    if let Some(t) = a.tag {
                        aenv.set_local(t, a.value.clone());
                        names.push(Some(t.name()));
                    }
                }
            }
            Value::List(l) => {
                let ns = obj.names().unwrap_or(Value::Null);
                let ns = crate::coerce::to_str(&ns);
                for (k, v) in l.data.iter().enumerate() {
                    let Some(n) = ns.get(k).and_then(|x| x.clone()) else {
                        return Err(Flow::error("not a valid named list"));
                    };
                    aenv.set_local(crate::sym::Sym::new(&n), v.clone());
                    names.push(Some(n));
                }
            }
            _ => return Err(Flow::error("loaded data is not in pair list form")),
        }
        Ok(Value::strs(names))
    })();
    if !wasopen {
        io::close_conn(i, con)?;
    }
    r
}

/// `loadInfoFromConn2(con)` — the header after a save() magic number.
// R: saveload.c do_loadFromConn2 (PRIMVAL = 1)
fn do_load_info_from_conn2(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    use super::io;
    let con = io::connection_number(&arg(&args, 0))?;
    let (wasopen, ..) = io::conn_state(i, con)?;
    if !wasopen && !io::open_in_mode(i, con, "rb")? {
        return Err(Flow::error("cannot open the connection"));
    }
    let r = (|| -> RResult<Value> {
        if !io::conn_state(i, con)?.1 {
            return Err(Flow::error("connection not open for reading"));
        }
        if io::conn_state(i, con)?.3 {
            return Err(Flow::error("can only load() from a binary connection"));
        }
        let mut magic = [0u8; 5];
        let count = io::con_read_bytes(i, con, &mut magic)?;
        if count == 0 {
            return Err(Flow::error("no input is available"));
        }
        let ok = ["RDA2\n", "RDB2\n", "RDX2\n", "RDA3\n", "RDB3\n", "RDX3\n"]
            .iter()
            .any(|m| magic.starts_with(m.as_bytes()));
        if !ok {
            return Err(Flow::error(
                "the input does not start with a magic number compatible with loading from a connection",
            ));
        }
        let rest = io::con_read_all(i, con)?;
        crate::serialize::serialize_info(&rest)
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

// R: serialize.c:R_getVarsFromFrame
fn do_get_vars_from_frame(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let env = match arg(&args, 1) {
        Value::Null => return Err(Flow::error("use of NULL environment is defunct")),
        Value::Env(e) => e,
        _ => return Err(Flow::error("bad environment")),
    };
    let vars = match arg(&args, 0) {
        Value::Str(v) => v,
        _ => return Err(Flow::error("bad variable names")),
    };
    let force = as_logical(&arg(&args, 2)) == 1;
    let mut out = Vec::with_capacity(vars.data.len());
    for v in vars.data.iter() {
        let name = v.as_deref().unwrap_or("NA");
        let mut val = env
            .get_local(crate::sym::Sym::new(name))
            .ok_or_else(|| Flow::error(format!("object '{}' not found", name)))?;
        if force {
            if let Value::Promise(_) = val {
                let g = i.global.clone();
                val = i.eval(&val, &g)?;
            }
        }
        out.push(val);
    }
    let mut ans = Value::list(out);
    ans.set_attr_raw(crate::sym::s::NAMES, Value::Str(vars.clone()));
    Ok(ans)
}

// R: serialize.c:R_lazyLoadDBinsertValue
fn do_lazy_load_db_insert_value(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let value = arg(&args, 0);
    let file = arg(&args, 1);
    if as_integer(&arg(&args, 2)) != 0 {
        return Err(Flow::error("robigo: ascii lazy-load databases are not implemented yet"));
    }
    let compress = as_integer(&arg(&args, 3));
    let hook = arg(&args, 4);
    let version = crate::serialize::default_serialize_version();
    let bytes = if hook.is_null() {
        crate::serialize::serialize_version(&value, version)
    } else {
        // R: serialize.c:CallHook — FUN(x) evaluated in the global environment
        let g = i.global.clone();
        let mut call_hook = |x: &Value| -> RResult<Value> { i.call_value(&hook, vec![Arg::pos(x.clone())], &g) };
        crate::serialize::serialize_hooked(&value, version, &mut call_hook)?
    };
    let bytes = match compress {
        3 => compress3(&bytes),
        2 => return Err(Flow::error("robigo: bzip2 lazy-load databases are not implemented yet")),
        0 => bytes,
        _ => compress1(&bytes)?,
    };
    append_raw_to_file(&file, &bytes)
}

/// R: connections.c:R_compress1 — the length (big-endian), then zlib at
/// level 6.
fn compress1(data: &[u8]) -> RResult<Vec<u8>> {
    use std::io::Write;
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    let mut enc = flate2::write::ZlibEncoder::new(&mut out, flate2::Compression::new(6));
    enc.write_all(data)
        .and_then(|_| enc.finish().map(|_| ()))
        .map_err(|_| Flow::error("internal libdeflate error in R_compress1 with libdeflate"))?;
    Ok(out)
}

/// R: connections.c:R_compress3 — the length, 'Z' and a raw LZMA2 stream
/// (preset 6); stored as is ('0') if that would not be smaller.
fn compress3(data: &[u8]) -> Vec<u8> {
    use liblzma::stream::{Action, Filters, LzmaOptions, Status, Stream};
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    let encoded = (|| {
        let opts = LzmaOptions::new_preset(6).ok()?;
        let mut filters = Filters::new();
        filters.lzma2(&opts);
        let mut strm = Stream::new_raw_encoder(&filters).ok()?;
        // R does not let the output expand beyond the input + 5 bytes
        let mut buf = vec![0u8; data.len() + 5];
        match strm.process(data, &mut buf, Action::Finish) {
            Ok(Status::StreamEnd) if strm.total_in() as usize == data.len() => {
                buf.truncate(strm.total_out() as usize);
                Some(buf)
            }
            _ => None,
        }
    })();
    match encoded {
        Some(z) => {
            out.push(b'Z');
            out.extend(z);
        }
        None => {
            out.push(b'0');
            out.extend_from_slice(data);
        }
    }
    out
}

/// R: serialize.c:appendRawToFile — (position, length) of the bytes.
fn append_raw_to_file(file: &Value, bytes: &[u8]) -> RResult<Value> {
    let path = match file {
        Value::Str(s) if s.data.len() == 1 && s.data[0].is_some() => s.data[0].clone().unwrap(),
        _ => return Err(Flow::error("not a proper file name")),
    };
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&*path)
        .map_err(|e| Flow::error(format!("cannot open file '{}': {}", path, super::io::os_error_message(&e))))?;
    let pos = f.metadata().map(|m| m.len()).map_err(|_| Flow::error("could not determine file position"))?;
    f.write_all(bytes).map_err(|_| Flow::error("write failed"))?;
    Ok(Value::ints(vec![pos as i32, bytes.len() as i32]))
}

// R: serialize.c:do_lazyLoadDBflush — robigo reads entries without a cache
fn do_lazy_load_db_flush(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Null)
}
