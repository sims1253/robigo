//! serialize.c's stream API: R_InitOutPStream/R_Serialize. The host's
//! serializer (robigo-runtime serialize.rs) produces the bytes, which go
//! to the stream's callbacks as GNU R writes them (format line first).

use crate::host::op;
use crate::memory::*;
use robigo_core::Sexp;
use std::ffi::{c_int, c_void};

/// R_ext's struct R_outpstream_st (Rinternals.h), as packages see it.
#[repr(C)]
pub struct OutPStream {
    pub data: *mut c_void,
    pub type_: c_int,
    pub version: c_int,
    pub out_char: Option<unsafe extern "C" fn(*mut OutPStream, c_int)>,
    pub out_bytes: Option<unsafe extern "C" fn(*mut OutPStream, *mut c_void, c_int)>,
    pub hook: Option<unsafe extern "C" fn(Sexp, Sexp) -> Sexp>,
    pub hook_data: Sexp,
}

/// R_pstream_format_t
const ANY: c_int = 0;
const ASCII: c_int = 1;
const BINARY: c_int = 2;
const XDR: c_int = 3;

// R: serialize.c:R_InitOutPStream
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_InitOutPStream(
    stream: *mut OutPStream,
    data: *mut c_void,
    type_: c_int,
    version: c_int,
    out_char: Option<unsafe extern "C" fn(*mut OutPStream, c_int)>,
    out_bytes: Option<unsafe extern "C" fn(*mut OutPStream, *mut c_void, c_int)>,
    hook: Option<unsafe extern "C" fn(Sexp, Sexp) -> Sexp>,
    hook_data: Sexp,
) {
    let s = &mut *stream;
    s.data = data;
    s.type_ = type_;
    s.version = if version == 0 { 3 } else { version };
    s.out_char = out_char;
    s.out_bytes = out_bytes;
    s.hook = hook;
    s.hook_data = hook_data;
}

// R: serialize.c:R_Serialize
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_Serialize(s: Sexp, stream: *mut OutPStream) {
    let st = &mut *stream;
    if st.hook.is_some() {
        crate::errors::not_implemented("R_Serialize with a persistence hook")
    }
    let fmt = match st.type_ {
        ANY | XDR => XDR,
        BINARY => BINARY,
        ASCII => crate::errors::not_implemented("R_Serialize to an ASCII stream"),
        _ => crate::errors::error_str("unknown output format"),
    };
    let args = [s, crate::list::Rf_ScalarInteger(fmt), crate::list::Rf_ScalarInteger(st.version)];
    let bytes = Rf_protect(op("serialize\0", &args));
    let n = xlength(bytes) as usize;
    let p = bytes.std_data();
    let emit = |off: usize, len: usize| match st.out_bytes {
        Some(f) => f(stream, p.add(off) as *mut c_void, len as c_int),
        None => {
            if let Some(c) = st.out_char {
                for k in 0..len {
                    c(stream, *p.add(off + k) as c_int);
                }
            }
        }
    };
    // The header in the pieces serialize.c writes it (OutFormat, then
    // OutInteger x3, and for version 3 the native encoding's length and
    // name): callbacks such as digest's skip it by counting bytes.
    let mut pieces = vec![2usize, 4, 4, 4];
    if st.version == 3 && n >= 18 {
        let raw = std::slice::from_raw_parts(p.add(14), 4);
        let nelen = if fmt == XDR {
            u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]])
        } else {
            u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])
        } as usize;
        pieces.extend([4, nelen]);
    }
    let mut off = 0;
    for len in pieces {
        if off + len > n {
            break;
        }
        emit(off, len);
        off += len;
    }
    // The items: serialize.c writes every integer field (flags, lengths)
    // as its own 4-byte piece; callbacks that skip a prefix rely on it
    // (digest's spookyhash skips 30 bytes into a character vector). Until
    // the host serializer reports its write boundaries, emit 4-byte
    // pieces: the same boundaries for all integer fields, other bodies
    // split finer than GNU R does (invisible to byte-stream consumers).
    while off < n {
        let len = (n - off).min(4);
        emit(off, len);
        off += len;
    }
    Rf_unprotect(1);
}
