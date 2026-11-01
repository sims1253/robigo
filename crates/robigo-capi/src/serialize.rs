//! serialize.c's stream API: R_InitOutPStream/R_Serialize. The host's
//! serializer (robigo-runtime serialize.rs) produces the bytes, which go
//! to the stream's callbacks as GNU R writes them (format line first).

use crate::host::op;
use crate::memory::*;
use crate::state::R_NilValue;
use robigo_core::Sexp;
use robigo_core::types::RAWSXP;
use std::ffi::{c_char, c_int, c_void};

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

/// R_ext's struct R_inpstream_st (Rinternals.h), as packages see it.
#[repr(C)]
pub struct InPStream {
    pub data: *mut c_void,
    pub type_: c_int,
    pub in_char: Option<unsafe extern "C" fn(*mut InPStream) -> c_int>,
    pub in_bytes: Option<unsafe extern "C" fn(*mut InPStream, *mut c_void, c_int)>,
    pub hook: Option<unsafe extern "C" fn(Sexp, Sexp) -> Sexp>,
    pub hook_data: Sexp,
    pub native_encoding: [c_char; 64],
    pub nat2nat_obj: *mut c_void,
    pub nat2utf8_obj: *mut c_void,
}

/// R_pstream_format_t
const ANY: c_int = 0;
const ASCII: c_int = 1;
const BINARY: c_int = 2;
const XDR: c_int = 3;

// R: serialize.c:R_InitInPStream
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_InitInPStream(
    stream: *mut InPStream,
    data: *mut c_void,
    type_: c_int,
    in_char: Option<unsafe extern "C" fn(*mut InPStream) -> c_int>,
    in_bytes: Option<unsafe extern "C" fn(*mut InPStream, *mut c_void, c_int)>,
    hook: Option<unsafe extern "C" fn(Sexp, Sexp) -> Sexp>,
    hook_data: Sexp,
) {
    let s = &mut *stream;
    s.data = data;
    s.type_ = type_;
    s.in_char = in_char;
    s.in_bytes = in_bytes;
    s.hook = hook;
    s.hook_data = hook_data;
    s.native_encoding[0] = 0;
    s.nat2nat_obj = (-1isize) as *mut c_void;
    s.nat2utf8_obj = (-1isize) as *mut c_void;
}

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

struct InStreamScanner {
    st: *mut InPStream,
    buf: Vec<u8>,
    be: bool,
    ascii: bool,
}

impl InStreamScanner {
    unsafe fn read_bytes(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        let start = self.buf.len();
        self.buf.resize(start + n, 0);
        if let Some(f) = (*self.st).in_bytes {
            f(self.st, self.buf.as_mut_ptr().add(start) as *mut c_void, n as c_int);
        } else if let Some(c) = (*self.st).in_char {
            for k in 0..n {
                let ch = c(self.st);
                if ch < 0 {
                    crate::errors::error_str("read error");
                }
                self.buf[start + k] = ch as u8;
            }
        } else {
            crate::errors::error_str("read error");
        }
    }

    unsafe fn read_word(&mut self) -> String {
        let Some(c) = (*self.st).in_char else {
            crate::errors::error_str("read error");
        };
        let mut word = Vec::new();
        loop {
            let ch = c(self.st);
            if ch < 0 {
                crate::errors::error_str("read error");
            }
            let b = ch as u8;
            self.buf.push(b);
            if !b.is_ascii_whitespace() {
                word.push(b);
                break;
            }
        }
        loop {
            let ch = c(self.st);
            if ch < 0 {
                break;
            }
            let b = ch as u8;
            self.buf.push(b);
            if b.is_ascii_whitespace() {
                break;
            }
            word.push(b);
        }
        String::from_utf8_lossy(&word).into_owned()
    }

    unsafe fn read_int(&mut self) -> i32 {
        if self.ascii {
            let w = self.read_word();
            if w == "NA" {
                return i32::MIN;
            }
            return w.parse().unwrap_or_else(|_| crate::errors::error_str("read error"));
        }
        let pos = self.buf.len();
        self.read_bytes(4);
        let b = [self.buf[pos], self.buf[pos + 1], self.buf[pos + 2], self.buf[pos + 3]];
        if self.be { i32::from_be_bytes(b) } else { i32::from_le_bytes(b) }
    }

    unsafe fn read_length(&mut self) -> usize {
        let len = self.read_int();
        if len < -1 {
            crate::errors::error_str("negative serialized length for vector");
        }
        if len == -1 {
            let hi = self.read_int() as u32 as u64;
            let lo = self.read_int() as u32 as u64;
            ((hi << 32) + lo) as usize
        } else {
            len as usize
        }
    }

    unsafe fn scan_charsxp(&mut self) {
        let len = self.read_int();
        if len > 0 {
            if self.ascii {
                let _ = self.read_word();
            } else {
                self.read_bytes(len as usize);
            }
        }
    }

    unsafe fn scan_string_vec(&mut self) {
        let _ = self.read_int();
        let len = self.read_int();
        for _ in 0..len.max(0) {
            let _ = self.read_int();
            self.scan_charsxp();
        }
    }

    unsafe fn scan_frame(&mut self) {
        loop {
            let flags = self.read_int() as u32;
            if flags & 255 != 2 {
                self.scan_item_flags(flags);
                return;
            }
            if flags & (1 << 9) != 0 {
                self.scan_item();
            }
            if flags & (1 << 10) != 0 {
                self.scan_item();
            }
            self.scan_item();
        }
    }

    unsafe fn scan_bc_lang(&mut self, ty: u32) {
        match ty {
            243 => {
                let _ = self.read_int();
            }
            244 | 6 | 2 | 240 | 239 => {
                let mut t = ty;
                if t == 244 {
                    let _ = self.read_int();
                    t = self.read_int() as u32;
                }
                if matches!(t, 240 | 239) {
                    self.scan_item();
                }
                self.scan_item();
                let t1 = self.read_int() as u32;
                self.scan_bc_lang(t1);
                let t2 = self.read_int() as u32;
                self.scan_bc_lang(t2);
            }
            _ => self.scan_item(),
        }
    }

    unsafe fn scan_bc(&mut self) {
        self.scan_item();
        let n = self.read_int();
        for _ in 0..n.max(0) {
            let ty = self.read_int() as u32;
            match ty {
                21 => self.scan_bc(),
                6 | 2 | 244 | 243 | 240 | 239 => self.scan_bc_lang(ty),
                _ => self.scan_item(),
            }
        }
    }

    unsafe fn scan_item(&mut self) {
        let flags = self.read_int() as u32;
        self.scan_item_flags(flags);
    }

    unsafe fn scan_item_flags(&mut self, flags: u32) {
        let ty = flags & 255;
        let hasattr = flags & (1 << 9) != 0;
        match ty {
            254 | 253 | 252 | 251 | 250 | 242 | 241 => return,
            255 => {
                if (flags >> 8) == 0 {
                    let _ = self.read_int();
                }
                return;
            }
            247 | 248 | 249 => {
                self.scan_string_vec();
                return;
            }
            238 => {
                self.scan_item();
                self.scan_item();
                self.scan_item();
                return;
            }
            1 => {
                self.scan_item();
                return;
            }
            4 => {
                let _locked = self.read_int();
                self.scan_item();
                self.scan_frame();
                let hflags = self.read_int() as u32;
                if hflags & 255 == 19 {
                    let n = self.read_length();
                    for _ in 0..n {
                        self.scan_frame();
                    }
                    if hflags & (1 << 9) != 0 {
                        self.scan_item();
                    }
                } else {
                    self.scan_item_flags(hflags);
                }
                self.scan_item();
                return;
            }
            2 | 6 | 3 | 5 | 17 => {
                let mut f = flags;
                loop {
                    let t = f & 255;
                    if !matches!(t, 2 | 6 | 3 | 5 | 17) {
                        self.scan_item_flags(f);
                        return;
                    }
                    if f & (1 << 9) != 0 {
                        self.scan_item();
                    }
                    if f & (1 << 10) != 0 {
                        self.scan_item();
                    }
                    self.scan_item();
                    if matches!(t, 3 | 5) {
                        self.scan_item();
                        return;
                    }
                    f = self.read_int() as u32;
                }
            }
            22 => {
                self.scan_item();
                self.scan_item();
            }
            23 | 25 | 0 => {}
            7 | 8 => {
                let len = self.read_int();
                if len > 0 {
                    if self.ascii {
                        let _ = self.read_word();
                    } else {
                        self.read_bytes(len as usize);
                    }
                }
            }
            9 => self.scan_charsxp(),
            10 | 13 => {
                let n = self.read_length();
                if self.ascii {
                    for _ in 0..n {
                        let _ = self.read_word();
                    }
                } else {
                    self.read_bytes(n * 4);
                }
            }
            14 => {
                let n = self.read_length();
                if self.ascii {
                    for _ in 0..n {
                        let _ = self.read_word();
                    }
                } else {
                    self.read_bytes(n * 8);
                }
            }
            15 => {
                let n = self.read_length();
                if self.ascii {
                    for _ in 0..n * 2 {
                        let _ = self.read_word();
                    }
                } else {
                    self.read_bytes(n * 16);
                }
            }
            24 => {
                let n = self.read_length();
                if self.ascii {
                    for _ in 0..n {
                        let _ = self.read_word();
                    }
                } else {
                    self.read_bytes(n);
                }
            }
            16 | 19 | 20 => {
                let n = self.read_length();
                for _ in 0..n {
                    self.scan_item();
                }
            }
            21 => {
                let _nreps = self.read_int();
                self.scan_bc();
            }
            _ => crate::errors::error_str("ReadItem: unknown type"),
        }
        if hasattr {
            self.scan_item();
        }
    }
}

// R: serialize.c:R_Unserialize
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_Unserialize(stream: *mut InPStream) -> Sexp {
    let st = &mut *stream;
    if st.hook.is_some() {
        crate::errors::not_implemented("R_Unserialize with a persistence hook")
    }
    let mut scanner = InStreamScanner { st: stream, buf: Vec::with_capacity(64), be: true, ascii: false };
    scanner.read_bytes(2);
    let fmt = match (scanner.buf[0], scanner.buf[1]) {
        (b'A', _) => {
            scanner.ascii = true;
            ASCII
        }
        (b'B', _) => {
            scanner.be = false;
            BINARY
        }
        (b'X', _) => {
            scanner.be = true;
            XDR
        }
        (b'\n', b'A') => {
            scanner.read_bytes(1);
            scanner.ascii = true;
            ASCII
        }
        _ => crate::errors::error_str("unknown input format"),
    };
    if st.type_ == ANY {
        st.type_ = fmt;
    } else if st.type_ != fmt {
        crate::errors::error_str("input format does not match specified format");
    }
    let version = scanner.read_int();
    let _writer = scanner.read_int();
    let _min_reader = scanner.read_int();
    if version == 3 {
        let nelen = scanner.read_int();
        if (0..=200).contains(&nelen) {
            if scanner.ascii {
                if nelen > 0 {
                    let _ = scanner.read_word();
                }
            } else if nelen > 0 {
                let name_start = scanner.buf.len();
                scanner.read_bytes(nelen as usize);
                let copy_len = (nelen as usize).min(63);
                for i in 0..copy_len {
                    st.native_encoding[i] = scanner.buf[name_start + i] as c_char;
                }
                st.native_encoding[copy_len] = 0;
            }
        }
    }
    if version == 2 || version == 3 {
        scanner.scan_item();
    }
    let raw = Rf_protect(Rf_allocVector(RAWSXP as u32, scanner.buf.len() as isize));
    std::ptr::copy_nonoverlapping(scanner.buf.as_ptr(), RAW(raw), scanner.buf.len());
    let res = crate::sort::internal("unserialize", &[raw, R_NilValue]);
    Rf_unprotect(1);
    res
}
