//! serialize.c (L1): GNU R's serialization, its ascii, binary and xdr
//! formats in versions 2 and 3 ([`stream`]), `WriteItem` ([`write`]) and
//! `ReadItem` ([`read`]) with the reference table, the persistent-name
//! hooks, byte code (`WriteBC`, `ReadBC`: the shared language cells as
//! `BCREPDEF`/`BCREPREF`, L1b over C1-BC's BCODESXP) and ALTREP's
//! serialized classes; `R_compile_pkgs`' compile of each closure as it
//! is written (`R_cmpfun1`: the compiler package's `tryCmpfun`), and of a
//! closure robigo-rt's JIT compiled, whose BODY GNU R's JIT has made that
//! byte code; and its builtins: `serialize`,
//! `serializeb`, `unserialize`, `serializeToConn`, `unserializeFromConn`,
//! `serializeInfoFromConn`, and the lazy-load database's
//! `lazyLoadDBinsertValue`, `lazyLoadDBfetch`, `lazyLoadDBflush` and
//! `getVarsFromFrame`.
//!
//! Not yet, and saying so when reached (AGENTS rule 6): a promise
//! robigo-rt's compiled code made, where GNU R's holds the argument's byte
//! code (MAKEPROM's, compiled with its closure: there is no BCODESXP of
//! GNU R's to write; one whose code is a BCODESXP is written as it is),
//! package ALTREP classes
//! (robigo-rtapi's; base's compact sequences are robigo-rt's own,
//! `robigo_rtcore::altrep::own`, written and read as GNU R writes them),
//! bzip2 and xz compressed databases, and native encodings other than
//! UTF-8, Latin-1 and ASCII.

#![allow(unsafe_code)]

pub mod read;
pub mod stream;
pub mod write;

use crate::connections;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Ref, Root, View};
use stream::{InStream, OutStream};

pub mod saveload;

/// `R_pstream_format_t`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Any,
    Ascii,
    AsciiHex,
    Binary,
    Xdr,
}

// serialize.c's administrative types
pub(crate) const REFSXP: i32 = 255;
pub(crate) const NILVALUE_SXP: i32 = 254;
pub(crate) const GLOBALENV_SXP: i32 = 253;
pub(crate) const UNBOUNDVALUE_SXP: i32 = 252;
pub(crate) const MISSINGARG_SXP: i32 = 251;
pub(crate) const BASENAMESPACE_SXP: i32 = 250;
pub(crate) const NAMESPACESXP: i32 = 249;
pub(crate) const PACKAGESXP: i32 = 248;
pub(crate) const PERSISTSXP: i32 = 247;
pub(crate) const CLASSREFSXP: i32 = 246;
pub(crate) const GENERICREFSXP: i32 = 245;
pub(crate) const BCREPDEF: i32 = 244;
pub(crate) const BCREPREF: i32 = 243;
pub(crate) const EMPTYENV_SXP: i32 = 242;
pub(crate) const BASEENV_SXP: i32 = 241;
pub(crate) const ATTRLANGSXP: i32 = 240;
pub(crate) const ATTRLISTSXP: i32 = 239;
pub(crate) const ALTREP_SXP: i32 = 238;

pub(crate) const IS_OBJECT_BIT_MASK: i32 = 1 << 8;
pub(crate) const HAS_ATTR_BIT_MASK: i32 = 1 << 9;
pub(crate) const HAS_TAG_BIT_MASK: i32 = 1 << 10;
pub(crate) const MAX_PACKED_INDEX: i32 = i32::MAX >> 8;

/// `R_Version(v, p, s)`.
pub(crate) const fn r_version(v: i32, p: i32, s: i32) -> i32 {
    v * 65536 + p * 256 + s
}

/// `R_VERSION`: 4.6.2.
pub(crate) const R_VERSION: i32 = r_version(4, 6, 2);

/// `R_nativeEncoding()`: the locale's codeset (`nl_langinfo(CODESET)`).
pub(crate) fn native_encoding(cx: &Cx) -> &'static str {
    if cx.locale().utf8 { "UTF-8" } else { "ANSI_X3.4-1968" }
}

/// `defaultSerializeVersion()`: `R_DEFAULT_SERIALIZE_VERSION` if 2 or 3,
/// else 3.
// R: serialize.c:defaultSerializeVersion
pub(crate) fn default_serialize_version() -> i32 {
    std::env::var("R_DEFAULT_SERIALIZE_VERSION")
        .ok()
        .map(|v| atoi(v.as_bytes()))
        .filter(|v| matches!(v, 2 | 3))
        .unwrap_or(3)
}

fn atoi(b: &[u8]) -> i32 {
    let s = std::str::from_utf8(b).unwrap_or("").trim_start();
    let end = s
        .char_indices()
        .take_while(|&(i, c)| c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')))
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    s[..end].parse().unwrap_or(0)
}

/// Whether `s` is one of the environments `SaveSpecialHook` names (and
/// `GetPersistentName` passes over).
fn special_env(cx: &Cx, s: Sexp) -> bool {
    s == cx.global_env().sexp() || s == cx.base_env().sexp() || s == cx.empty_env().sexp()
}

/// `SaveSpecialHook(item)`.
// R: serialize.c:SaveSpecialHook
fn save_special_hook(cx: &Cx, s: Sexp) -> Option<i32> {
    Some(if s == cx.nil().sexp() {
        NILVALUE_SXP
    } else if s == cx.empty_env().sexp() {
        EMPTYENV_SXP
    } else if s == cx.base_env().sexp() {
        BASEENV_SXP
    } else if s == cx.global_env().sexp() {
        GLOBALENV_SXP
    } else if s == cx.unbound().sexp() {
        UNBOUNDVALUE_SXP
    } else if s == cx.missing_arg().sexp() {
        MISSINGARG_SXP
    } else if s == cx.base_namespace().sexp() {
        BASENAMESPACE_SXP
    } else {
        return None;
    })
}

// What a namespace or package environment is: robigo-rt's `ns`
// (envir.c's R_IsNamespaceEnv, R_NamespaceEnvSpec, R_IsPackageEnv).
use crate::ns::{is_namespace_env, is_package_env, name_attr, namespace_spec};

/// `R_NamespaceEnvSpec(rho)` of a namespace other than base's.
///
/// # Safety
/// `s` is a live namespace environment (not base's).
unsafe fn namespace_env_spec(cx: &Cx, s: Sexp) -> Sexp {
    namespace_spec(cx, s).expect("robigo: not a namespace environment")
}

/// `R_PackageEnvName(rho)`.
///
/// # Safety
/// `s` is a live package environment.
// R: envir.c:R_PackageEnvName
unsafe fn package_env_name(cx: &Cx, s: Sexp) -> Sexp {
    name_attr(cx, s).expect("robigo: not a package environment")
}

/// The connection stream's read: exactly `buf.len()` bytes (`InBytesConn`).
pub(crate) fn conn_read_raw(cx: &Cx, n: i32, buf: &mut [u8]) -> Result<(), String> {
    connections::read_raw(cx, n, buf)
}

// ---- argument helpers ---------------------------------------------------------

fn version_arg(cx: &mut Cx, v: Root<'_>) -> RResult<i32> {
    let version = if v.bind(cx).sexptype() == NILSXP {
        default_serialize_version()
    } else {
        robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, v)?
    };
    if version == i32::MIN || version <= 0 {
        return Err(cx.error("bad version value"));
    }
    Ok(version)
}

fn hook_arg<'s>(cx: &Cx, f: Root<'s>) -> Option<Root<'s>> {
    (f.bind(cx).sexptype() != NILSXP).then_some(f)
}

/// The raw vector of `bytes`.
fn raw_vector(cx: &mut Cx, bytes: &[u8], out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| Ok(cx.alloc_copy(bytes)?.finish().into()))
}

/// `R_InitConnOutPStream`'s checks, then `R_Serialize` to connection `n`.
// R: serialize.c:R_InitConnOutPStream, CheckOutConn
fn serialize_to_conn(
    cx: &mut Cx,
    n: i32,
    object: Root<'_>,
    fmt: Format,
    version: i32,
    hook: Option<Root<'_>>,
) -> RResult<()> {
    let f = connections::flags(cx, n)?;
    if !f.isopen {
        return Err(cx.error("connection is not open"));
    }
    if !f.canwrite {
        return Err(cx.error("cannot write to this connection"));
    }
    if f.text && !matches!(fmt, Format::Ascii | Format::AsciiHex) {
        return Err(cx.error("only ascii format can be written to text mode connections"));
    }
    let bytes = write::serialize(cx, object, OutStream::new(fmt, version), hook)?;
    connections::write_all(cx, n, &bytes, "error writing to connection")
}

/// `R_InitConnInPStream`'s checks: the stream over connection `n`.
// R: serialize.c:R_InitConnInPStream, CheckInConn
fn conn_in_stream(cx: &mut Cx, n: i32) -> RResult<InStream> {
    let f = connections::flags(cx, n)?;
    if !f.isopen {
        return Err(cx.error("connection is not open"));
    }
    if !f.canread {
        return Err(cx.error("cannot read from this connection"));
    }
    let mut s = InStream::conn(n, f.text);
    if f.text {
        s.fmt = Format::Ascii;
    }
    Ok(s)
}

// ---- the builtins -------------------------------------------------------------

/// `serialize(object, connection, type, version, hook)` (CODE 0),
/// `serializeb(...)` (1) and `unserialize(connection, hook)` (2).
// R: serialize.c:do_serialize, R_serialize, R_serializeb, R_unserialize
pub fn do_serialize<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let name = ["serialize", "serializeb", "unserialize"][CODE as usize];
    crate::prims::check_arity(cx, call, args.len(), name, if CODE == 2 { 2 } else { 5 }, true)?;
    if CODE == 2 {
        let icon = args.root(0);
        let hook = hook_arg(cx, args.root(1));
        let v = cx.scope(|cx, s| {
            let o = s.out(cx);
            let r = o.root();
            unserialize_from(cx, icon, hook, o)?;
            Ok::<_, robigo_rtcore::RError>(r.bind(cx).sexp())
        })?;
        // SAFETY: the result was rooted until the scope ended, and nothing
        // has run since.
        let v = unsafe { Ref::from_sexp(v, cx) };
        if v.sexptype() == PROMSXP {
            return Err(cx.error("cannot return a promise (PROMSXP) object"));
        }
        out.set(cx, v);
        return Ok(());
    }
    let (object, icon, ty, ver) = (args.root(0), args.root(1), args.root(2), args.root(3));
    let hook = hook_arg(cx, args.root(4));
    if CODE == 1 {
        let n = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, icon)?;
        connections::with_conn(cx, n, |_| ())?;
        let version = version_arg(cx, ver)?;
        let xdr = robigo_rtbuiltins::util::as_logical(cx, ty.bind(cx));
        let fmt = if xdr != 0 { Format::Xdr } else { Format::Binary };
        let bytes = write::serialize(cx, object, OutStream::new(fmt, version), hook)?;
        // (OutBytesBB's buffer of BCONBUFSIZ, written in pieces)
        for chunk in bytes.chunks(4096) {
            connections::write_all(cx, n, chunk, "error writing to connection")?;
        }
        out.set(cx, cx.nil());
        return Ok(());
    }
    let version = version_arg(cx, ver)?;
    let asc = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, ty)?;
    let fmt = match asc {
        1 => Format::Ascii,
        2 => Format::AsciiHex,
        3 => Format::Binary,
        _ => Format::Xdr,
    };
    if icon.bind(cx).sexptype() == NILSXP {
        let bytes = write::serialize(cx, object, OutStream::new(fmt, version), hook)?;
        if bytes.len() > isize::MAX as usize {
            return Err(cx.error("serialization is too large to store in a raw vector"));
        }
        return raw_vector(cx, &bytes, out);
    }
    let n = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, icon)?;
    serialize_to_conn(cx, n, object, fmt, version, hook)?;
    out.set(cx, cx.nil());
    Ok(())
}

/// `R_unserialize(icon, fun)`: from a raw vector or a connection.
// R: serialize.c:R_unserialize
fn unserialize_from(cx: &mut Cx, icon: Root<'_>, hook: Option<Root<'_>>, out: Out<'_>) -> RResult<()> {
    let r = icon.bind(cx);
    match r.view() {
        View::Str(v) if !v.is_empty() => Err(cx.error("character vectors are no longer accepted by unserialize()")),
        View::Raw(v) => {
            let data = v.as_slice().map(|s| s.to_vec());
            let Some(data) = data else {
                return Err(cx.error("robigo: unserializing a package ALTREP raw vector is not implemented yet"));
            };
            read::unserialize(cx, InStream::memory(data), hook, out)
        }
        _ => {
            let n = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, icon)?;
            connections::with_conn(cx, n, |_| ())?;
            let s = conn_in_stream(cx, n)?;
            read::unserialize(cx, s, hook, out)
        }
    }
}

/// `serializeToConn(object, conn, ascii, version, hook)`, saveRDS's.
// R: serialize.c:do_serializeToConn
pub fn do_serialize_to_conn(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "serializeToConn", 5, true)?;
    let object = args.root(0);
    let n = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(1))?;
    connections::with_conn(cx, n, |_| ())?;
    let ascii = {
        let v = robigo_rtbuiltins::util::as_logical_n(cx, args.get(cx, 2));
        // asRbool(x, call): NA is kept (the next line's asciihex)
        if robigo_rtbuiltins::util::is_vector_atomic(args.get(cx, 2)) && v.1 > 1 {
            let msg = format!("'length = {}' in coercion to 'logical(1)'", v.1);
            return Err(cx.error_call(Some(call), &msg));
        }
        v.0
    };
    let fmt = if ascii == i32::MIN {
        Format::AsciiHex
    } else if ascii != 0 {
        Format::Ascii
    } else {
        Format::Xdr
    };
    let ver = args.root(3);
    let version = if ver.bind(cx).sexptype() == NILSXP {
        default_serialize_version()
    } else {
        robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, ver)?
    };
    if version == i32::MIN || version <= 0 {
        return Err(cx.error("bad version value"));
    }
    if version < 2 {
        return Err(cx.error(&format!("cannot save to connections in version {version} format")));
    }
    let hook = hook_arg(cx, args.root(4));
    let wasopen = connections::flags(cx, n)?.isopen;
    if !wasopen && !connections::open_with_mode(cx, n, if ascii != 0 { "w" } else { "wb" })? {
        return Err(cx.error("cannot open the connection"));
    }
    let r = (|| {
        let f = connections::flags(cx, n)?;
        if ascii == 0 && f.text {
            return Err(cx.error("binary-mode connection required for ascii=false"));
        }
        if !f.canwrite {
            return Err(cx.error("connection not open for writing"));
        }
        serialize_to_conn(cx, n, object, fmt, version, hook)
    })();
    if !wasopen {
        connections::close_only(cx, n)?;
    }
    r?;
    out.set(cx, cx.nil());
    Ok(())
}

/// `unserializeFromConn(conn, hook)` (CODE 0, readRDS's) and
/// `serializeInfoFromConn(conn)` (1).
// R: serialize.c:do_unserializeFromConn
pub fn do_unserialize_from_conn<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let name = ["unserializeFromConn", "serializeInfoFromConn"][CODE as usize];
    crate::prims::check_arity(cx, call, args.len(), name, if CODE == 0 { 2 } else { 1 }, true)?;
    let n = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(0))?;
    connections::with_conn(cx, n, |_| ())?;
    let wasopen = connections::flags(cx, n)?.isopen;
    if !wasopen && !connections::open_with_mode(cx, n, "rb")? {
        return Err(cx.error("cannot open the connection"));
    }
    let hook = if CODE == 0 { hook_arg(cx, args.root(1)) } else { None };
    let r = cx.scope(|cx, s| {
        if !connections::flags(cx, n)?.canread {
            return Err(cx.error("connection not open for reading"));
        }
        let st = conn_in_stream(cx, n)?;
        let o = s.out(cx);
        let r = o.root();
        if CODE == 0 {
            read::unserialize(cx, st, hook, o)?;
        } else {
            read::serialize_info(cx, st, o)?;
        }
        Ok(r.bind(cx).sexp())
    });
    if !wasopen {
        connections::close_only(cx, n)?;
    }
    let v = r?;
    // SAFETY: rooted until the scope ended; closing a connection runs no R
    // code and does not collect.
    let v = unsafe { Ref::from_sexp(v, cx) };
    if v.sexptype() == PROMSXP {
        return Err(cx.error("cannot return a promise (PROMSXP) object"));
    }
    out.set(cx, v);
    Ok(())
}

// ---- lazy-load databases ------------------------------------------------------

/// The `.rdb` files read whole (serialize.c's `names`/`ptr` cache: up to
/// 100 files under 10 MB, kept until `lazyLoadDBflush`).
#[derive(Default)]
pub struct RdbCache {
    files: std::cell::RefCell<Vec<Option<(Vec<u8>, Vec<u8>)>>>,
}

const NC: usize = 100;
const LEN_LIMIT: u64 = 10 * 1048576;

/// `IS_PROPER_STRING(file)`'s first element, translated (`translateCharFP`).
fn proper_file(cx: &mut Cx, file: Root<'_>) -> RResult<Vec<u8>> {
    let c = match file.bind(cx).view() {
        View::Str(v) if !v.is_empty() => Some(v.get(0)),
        _ => None,
    };
    let Some(c) = c else { return Err(cx.error("not a proper file name")) };
    let b = robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
        .map(|b| b.into_owned())
        .unwrap_or_else(|_| c.bytes().to_vec());
    Ok(b)
}

fn rdb_cache(cx: &Cx) -> &RdbCache {
    cx.ext::<RdbCache>().expect("robigo: the lazy-load cache is not installed (robigo_rt::install)")
}

/// `readRawFromFile(file, key)`: the bytes at `key`'s offset and length.
// R: serialize.c:readRawFromFile
fn read_raw_from_file(cx: &mut Cx, file: Root<'_>, key: Root<'_>) -> RResult<Vec<u8>> {
    let cfile = proper_file(cx, file)?;
    let k = key.bind(cx);
    let (offset, len) = match k.view() {
        View::Int(v) if v.len() == 2 => (v.get(0), v.get(1)),
        _ => return Err(cx.error("bad offset/length argument")),
    };
    if len < 0 {
        return Err(cx.error("negative length vectors are not allowed"));
    }
    let (offset, len) = (offset as i64, len as usize);
    let shown = String::from_utf8_lossy(&cfile).into_owned();
    let path = std::path::PathBuf::from(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&cfile));
    let cached = {
        let c = rdb_cache(cx).files.borrow();
        c.iter().flatten().find(|(n, _)| *n == cfile).map(|(_, d)| slice_at(d, offset, len))
    };
    if let Some(v) = cached {
        return v.ok_or_else(|| cx.error(&format!("read failed on {shown}")));
    }
    let vacant = {
        let c = rdb_cache(cx).files.borrow();
        c.iter().position(|e| e.is_none()).or(if c.len() < NC { Some(c.len()) } else { None })
    };
    let mut f = std::fs::File::open(&path)
        .map_err(|e| cx.error(&format!("cannot open file '{shown}': {}", connections::strerror(&e))))?;
    use std::io::{Read, Seek, SeekFrom};
    let filelen = f.seek(SeekFrom::End(0)).map_err(|_| cx.error(&format!("seek failed on {shown}")))?;
    if let Some(i) = vacant
        && filelen < LEN_LIMIT
    {
        f.seek(SeekFrom::Start(0)).map_err(|_| cx.error(&format!("seek failed on {shown}")))?;
        let mut data = Vec::with_capacity(filelen as usize);
        f.read_to_end(&mut data).map_err(|_| cx.error(&format!("read failed on {shown}")))?;
        if data.len() as u64 != filelen {
            return Err(cx.error(&format!("read failed on {shown}")));
        }
        let v = slice_at(&data, offset, len);
        {
            let mut c = rdb_cache(cx).files.borrow_mut();
            if i == c.len() {
                c.push(None);
            }
            c[i] = Some((cfile.clone(), data));
        }
        return v.ok_or_else(|| cx.error(&format!("read failed on {shown}")));
    }
    f.seek(SeekFrom::Start(offset.max(0) as u64)).map_err(|_| cx.error(&format!("seek failed on {shown}")))?;
    let mut v = vec![0; len];
    let mut got = 0;
    while got < len {
        match f.read(&mut v[got..]) {
            Ok(0) => break,
            Ok(k) => got += k,
            Err(_) => break,
        }
    }
    if got != len {
        return Err(cx.error(&format!("read failed on {shown}")));
    }
    Ok(v)
}

fn slice_at(d: &[u8], offset: i64, len: usize) -> Option<Vec<u8>> {
    let o = usize::try_from(offset).ok()?;
    d.get(o..o.checked_add(len)?).map(|s| s.to_vec())
}

/// `R_decompress1(in, &err)`: a 4-byte big-endian length, then a zlib
/// stream (libdeflate's, in GNU R's build: its error numbers).
// R: connections.c:R_decompress1
fn decompress1(cx: &mut Cx, data: &[u8]) -> RResult<Option<Vec<u8>>> {
    if data.len() < 4 {
        cx.warning("internal error 1 in R_decompress1 with libdeflate")?;
        return Ok(None);
    }
    let outlen = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let mut buf = vec![0u8; outlen];
    let mut d = flate2::Decompress::new(true);
    let r = d.decompress(&data[4..], &mut buf, flate2::FlushDecompress::Finish);
    let code = match r {
        Ok(flate2::Status::StreamEnd) => 0,
        Ok(_) if d.total_out() as usize == outlen => 3,
        _ => 1,
    };
    if code != 0 {
        cx.warning(&format!("internal error {code} in R_decompress1 with libdeflate"))?;
        return Ok(None);
    }
    buf.truncate(d.total_out() as usize);
    Ok(Some(buf))
}

/// `R_compress1(in)`: the length, big-endian, then the zlib stream at
/// level 6.
// R: connections.c:R_compress1
fn compress1(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2 + 64);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut c = flate2::Compress::new(flate2::Compression::new(6), true);
    let mut buf = Vec::with_capacity(data.len() + data.len() / 1000 + 64);
    loop {
        let st = c.compress_vec(&data[c.total_in() as usize..], &mut buf, flate2::FlushCompress::Finish);
        match st {
            Ok(flate2::Status::StreamEnd) => break,
            Ok(_) => buf.reserve(buf.capacity().max(4096)),
            Err(_) => unreachable!("robigo: deflate of a buffer failed"),
        }
    }
    out.extend_from_slice(&buf);
    out
}

/// `lazyLoadDBfetch(key, file, compressed, hook)`: the value stored at
/// `key`, unserialized (a promise forced).
// R: serialize.c:do_lazyLoadDBfetch
pub fn do_lazyload_dbfetch(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "lazyLoadDBfetch", 4, false)?;
    let (key, file) = (args.root(0), args.root(1));
    let compressed = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(2))?;
    let hook = hook_arg(cx, args.root(3));
    let raw = read_raw_from_file(cx, file, key)?;
    let data = match compressed {
        0 => Some(raw),
        2 | 3 => {
            return Err(cx.error("robigo: bzip2 and xz compressed lazy-load databases are not implemented yet"));
        }
        _ => decompress1(cx, &raw)?,
    };
    let Some(data) = data else {
        let c = match file.bind(cx).view() {
            View::Str(v) => v.get(0).bytes().to_vec(),
            _ => Vec::new(),
        };
        let shown = String::from_utf8_lossy(&c).into_owned();
        return Err(cx.error(&format!("lazy-load database '{shown}' is corrupt")));
    };
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let v = o.root();
        read::unserialize(cx, InStream::memory(data), hook, o)?;
        if v.bind(cx).sexptype() == PROMSXP {
            let g = s.root(cx, cx.global_env());
            return cx.eval(v, g, out);
        }
        out.set(cx, v.bind(cx));
        Ok(())
    })
}

/// `lazyLoadDBflush(file)`: forget the cached file.
// R: serialize.c:do_lazyLoadDBflush
pub fn do_lazyload_dbflush(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "lazyLoadDBflush", 1, true)?;
    let name = match args.get(cx, 0).view() {
        View::Str(v) if !v.is_empty() => Some(v.get(0)),
        _ => None,
    };
    let Some(c) = name else { return Err(cx.error("robigo: lazyLoadDBflush of a non-string is not implemented yet")) };
    let cfile = robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
        .map(|b| b.into_owned())
        .unwrap_or_else(|_| c.bytes().to_vec());
    let mut files = rdb_cache(cx).files.borrow_mut();
    if let Some(e) = files.iter_mut().find(|e| e.as_ref().is_some_and(|(n, _)| *n == cfile)) {
        *e = None;
    }
    drop(files);
    out.set(cx, cx.nil());
    Ok(())
}

/// `lazyLoadDBinsertValue(value, file, ascii, compsxp, hook)`: serialize,
/// compress, append to `file`; the key `c(offset, length)`.
// R: serialize.c:do_lazyLoadDBinsertValue, R_lazyLoadDBinsertValue, appendRawToFile
pub fn do_lazyload_dbinsert_value(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "lazyLoadDBinsertValue", 5, true)?;
    let (value, file, ascii) = (args.root(0), args.root(1), args.root(2));
    let compress = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(3))?;
    let hook = hook_arg(cx, args.root(4));
    let version = default_serialize_version();
    let asc = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, ascii)?;
    let fmt = match asc {
        1 => Format::Ascii,
        2 => Format::AsciiHex,
        3 => Format::Binary,
        _ => Format::Xdr,
    };
    let bytes = write::serialize(cx, value, OutStream::new(fmt, version), hook)?;
    let bytes = match compress {
        0 => bytes,
        2 | 3 => {
            return Err(cx.error("robigo: bzip2 and xz compressed lazy-load databases are not implemented yet"));
        }
        _ => compress1(&bytes),
    };
    // appendRawToFile
    let ok = matches!(file.bind(cx).view(), View::Str(v) if !v.is_empty());
    if !ok {
        return Err(cx.error("not a proper file name"));
    }
    let cfile = proper_file(cx, file)?;
    let shown = String::from_utf8_lossy(&cfile).into_owned();
    let path = std::path::PathBuf::from(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&cfile));
    use std::io::{Seek, Write};
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .map_err(|e| cx.error(&format!("cannot open file '{shown}': {}", connections::strerror(&e))))?;
    let pos = f.seek(std::io::SeekFrom::End(0)).map_err(|_| cx.error("could not determine file position"))?;
    if f.write_all(&bytes).is_err() {
        return Err(cx.error("write failed"));
    }
    cx.build(out, |cx| Ok(cx.alloc_copy(&[pos as i32, bytes.len() as i32])?.finish().into()))
}

/// `getVarsFromFrame(vars, env, force)`: the bindings' values as a named
/// list, promises forced if `force`.
// R: serialize.c:do_getVarsFromFrame, R_getVarsFromFrame
pub fn do_get_vars_from_frame(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "getVarsFromFrame", 3, true)?;
    let (vars, env) = (args.root(0), args.root(1));
    match env.bind(cx).sexptype() {
        NILSXP => return Err(cx.error("use of NULL environment is defunct")),
        ENVSXP => {}
        _ => return Err(cx.error("bad environment")),
    }
    let n = match vars.bind(cx).view() {
        View::Str(v) => v.len(),
        _ => return Err(cx.error("bad variable names")),
    };
    let force = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    if force == i32::MIN {
        return Err(cx.error_call(None, "NA in coercion to boolean"));
    }
    cx.scope(|cx, s| {
        let vals: Vec<Root<'_>> = (0..n).map(|_| s.out(cx).root()).collect();
        for (i, &slot) in vals.iter().enumerate() {
            cx.scope(|cx, t| {
                let c = vars.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
                let name = robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
                    .map(|b| b.into_owned())
                    .unwrap_or_else(|_| c.bytes().to_vec());
                let sym = cx.install_rooted(t, &name)?;
                let o = t.out(cx);
                let v = o.root();
                cx.find_var_in_frame3(env, sym, true, o)?;
                if v.bind(cx).sexp() == cx.unbound().sexp() {
                    let shown = String::from_utf8_lossy(&name).into_owned();
                    return Err(cx.error(&format!("object '{shown}' not found")));
                }
                if force != 0 && v.bind(cx).sexptype() == PROMSXP {
                    let g = t.root(cx, cx.global_env());
                    let o = t.out(cx);
                    let f = o.root();
                    cx.eval(v, g, o)?;
                    slot.replace_from(cx, f);
                } else {
                    slot.replace_from(cx, v);
                }
                Ok(())
            })?;
        }
        cx.build(out, |cx| {
            let mut list = cx.alloc_vector::<Ref<'_>>(n)?;
            for (i, r) in vals.iter().enumerate() {
                list.set(i, r.bind(cx));
            }
            let mut a = robigo_rtbuiltins::attrib::Attrs::new();
            a.install(robigo_rtbuiltins::attrib::Attr {
                tag: cx.sym(robigo_rtcore::RSym::Names),
                value: vars.bind(cx),
            });
            a.apply(cx, &mut list)?;
            Ok(list.finish().into())
        })
    })
}

/// The `Encoding` a CHARSXP read with gp `levs` is marked with, when the
/// marks decide it (`ReadChar`'s first tests).
fn marked_encoding(levs: u16) -> Option<Encoding> {
    use robigo_heap::gp::charsxp as gp;
    if levs & gp::UTF8 != 0 {
        Some(Encoding::Utf8)
    } else if levs & gp::LATIN1 != 0 {
        Some(Encoding::Latin1)
    } else if levs & gp::BYTES != 0 {
        Some(Encoding::Bytes)
    } else if levs & gp::ASCII != 0 {
        Some(Encoding::Native)
    } else {
        None
    }
}

// ---- for Rust callers ---------------------------------------------------------

/// `R_Unserialize` of `data` (a memory stream) with `hook` as the
/// persistent-name restore function (`unserialize(raw, refhook = hook)`),
/// written to `out`.
pub fn unserialize_bytes(cx: &mut Cx, data: Vec<u8>, hook: Option<Root<'_>>, out: Out<'_>) -> RResult<()> {
    read::unserialize(cx, InStream::memory(data), hook, out)
}

/// `R_Serialize` of `object` in `fmt` and `version` with `hook` as the
/// persistent-name function (`serialize(object, NULL, ..., refhook = hook)`).
pub fn serialize_bytes(
    cx: &mut Cx,
    object: Root<'_>,
    fmt: Format,
    version: i32,
    hook: Option<Root<'_>>,
) -> RResult<Vec<u8>> {
    write::serialize(cx, object, OutStream::new(fmt, version), hook)
}
