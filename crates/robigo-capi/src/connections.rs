//! R_ext/Connections.h: the C API to R connections, over the host's
//! connection table. C code gets a GNU R `struct Rconn` per connection
//! (the layout is public, `R_CONNECTIONS_VERSION` 1): its fields mirror
//! the host's connection and are refreshed whenever C asks for it or
//! calls `open`/`close`; its methods call the host.

use crate::host::op;
use crate::list::Rf_ScalarInteger;
use crate::memory::*;
use robigo_core::Sexp;
use robigo_core::types::*;
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_void};

type Method<R> = Option<unsafe extern "C" fn(*mut Rconn) -> R>;

/// GNU R 4.5's `struct Rconn` (R_ext/Connections.h; offsets checked
/// against the header by `tests/capi.rs`).
#[repr(C)]
pub struct Rconn {
    pub class: *mut c_char,
    pub description: *mut c_char,
    pub enc: c_int,
    pub mode: [c_char; 5],
    pub text: c_int,
    pub isopen: c_int,
    pub incomplete: c_int,
    pub canread: c_int,
    pub canwrite: c_int,
    pub canseek: c_int,
    pub blocking: c_int,
    pub is_gzcon: c_int,
    pub open: Method<c_int>,
    pub close: Method<()>,
    pub destroy: Method<()>,
    pub vfprintf: Option<unsafe extern "C" fn(*mut Rconn, *const c_char, *mut c_void) -> c_int>,
    pub fgetc: Method<c_int>,
    pub fgetc_internal: Method<c_int>,
    pub seek: Option<unsafe extern "C" fn(*mut Rconn, f64, c_int, c_int) -> f64>,
    pub truncate: Method<()>,
    pub fflush: Method<c_int>,
    pub read: Option<unsafe extern "C" fn(*mut c_void, usize, usize, *mut Rconn) -> usize>,
    pub write: Option<unsafe extern "C" fn(*const c_void, usize, usize, *mut Rconn) -> usize>,
    pub n_push_back: c_int,
    pub pos_push_back: c_int,
    pub push_back: *mut *mut c_char,
    pub save: c_int,
    pub save2: c_int,
    pub encname: [c_char; 101],
    pub inconv: *mut c_void,
    pub outconv: *mut c_void,
    pub iconvbuff: [c_char; 25],
    pub oconvbuff: [c_char; 50],
    pub next: *mut c_char,
    pub init_out: [c_char; 25],
    pub navail: i16,
    pub inavail: i16,
    pub eof_signalled: c_int,
    pub utf8out: c_int,
    pub id: *mut c_void,
    pub ex_ptr: *mut c_void,
    pub private: *mut c_void,
    pub status: c_int,
    pub buff: *mut u8,
    pub buff_len: usize,
    pub buff_stored_len: usize,
    pub buff_pos: usize,
}

struct Entry {
    n: c_int,
    /// the strings `class` and `description` point into
    strings: (CString, CString),
}

static mut CONNS: Option<HashMap<usize, Entry>> = None;
static mut BY_NUMBER: Option<HashMap<c_int, usize>> = None;

pub unsafe fn number_of(con: *mut c_void) -> c_int {
    match CONNS.as_ref().and_then(|m| m.get(&(con as usize))) {
        Some(e) => e.n,
        None => crate::errors::error_str("invalid connection"),
    }
}

/// Copy the host connection's fields into its `struct Rconn`.
unsafe fn refresh(con: *mut Rconn) {
    let n = number_of(con as *mut c_void);
    let info = Rf_protect(op("connInfo\0", &[Rf_ScalarInteger(n)]));
    let s = |k: isize| {
        let c = STRING_ELT(VECTOR_ELT(info, k), 0);
        CString::new(c.char_bytes().to_vec()).unwrap_or_default()
    };
    let (class, desc, mode) = (s(0), s(1), s(2));
    let flags = std::slice::from_raw_parts(LOGICAL(VECTOR_ELT(info, 3)), 7);
    Rf_unprotect(1);
    let c = &mut *con;
    c.class = class.as_ptr() as *mut c_char;
    c.description = desc.as_ptr() as *mut c_char;
    c.mode = [0; 5];
    for (k, &b) in mode.as_bytes().iter().take(4).enumerate() {
        c.mode[k] = b as c_char;
    }
    [c.text, c.isopen, c.incomplete, c.canread, c.canwrite, c.canseek, c.blocking] =
        [flags[0], flags[1], flags[2], flags[3], flags[4], flags[5], flags[6]];
    if let Some(e) = CONNS.as_mut().and_then(|m| m.get_mut(&(con as usize))) {
        e.strings = (class, desc);
    }
}

unsafe extern "C" fn m_open(con: *mut Rconn) -> c_int {
    let n = number_of(con as *mut c_void);
    let ok = *LOGICAL(op("connOpen\0", &[Rf_ScalarInteger(n)]));
    refresh(con);
    ok
}

unsafe extern "C" fn m_close(con: *mut Rconn) {
    let n = number_of(con as *mut c_void);
    op("connClose\0", &[Rf_ScalarInteger(n)]);
    refresh(con);
}

unsafe extern "C" fn m_destroy(_: *mut Rconn) {}

unsafe extern "C" {
    fn vasprintf(strp: *mut *mut c_char, fmt: *const c_char, ap: *mut c_void) -> c_int;
    fn robigo_connection_string_arg(fmt: *const c_char, ap: *mut c_void) -> *const c_char;
}

unsafe extern "C" fn m_vfprintf(con: *mut Rconn, fmt: *const c_char, ap: *mut c_void) -> c_int {
    // R: library/utils/src/io.c:writetable uses literal %s for cells and separators.
    // Null strings and counts outside printf's int range use libc unchanged.
    let arg = robigo_connection_string_arg(fmt, ap);
    if !arg.is_null() {
        let n = libc::strlen(arg);
        if n <= c_int::MAX as usize {
            // Copy before entering R_WriteConnection: its allocation may run
            // finalizers that overwrite a native formatter's static buffer.
            if n <= 256 {
                let mut scratch = std::mem::MaybeUninit::<[u8; 256]>::uninit();
                let dst = scratch.as_mut_ptr().cast::<u8>();
                std::ptr::copy_nonoverlapping(arg.cast::<u8>(), dst, n);
                R_WriteConnection(con as *mut c_void, dst.cast(), n);
            } else {
                let bytes = std::slice::from_raw_parts(arg.cast::<u8>(), n).to_vec();
                R_WriteConnection(con as *mut c_void, bytes.as_ptr().cast(), n);
            }
            return n as c_int;
        }
    }
    let mut s: *mut c_char = std::ptr::null_mut();
    let r = vasprintf(&mut s, fmt, ap);
    if r < 0 {
        return r;
    }
    let bytes = std::slice::from_raw_parts(s as *const u8, r as usize).to_vec();
    libc::free(s as *mut c_void);
    R_WriteConnection(con as *mut c_void, bytes.as_ptr() as *const c_void, bytes.len());
    r
}

unsafe extern "C" fn m_fgetc(con: *mut Rconn) -> c_int {
    Rconn_fgetc(con as *mut c_void)
}

unsafe extern "C" fn m_seek(_: *mut Rconn, _: f64, _: c_int, _: c_int) -> f64 {
    crate::errors::error_str("robigo: seeking a connection from C is not implemented yet")
}

unsafe extern "C" fn m_truncate(_: *mut Rconn) {
    crate::errors::error_str("robigo: truncating a connection from C is not implemented yet")
}

unsafe extern "C" fn m_fflush(_: *mut Rconn) -> c_int {
    0
}

unsafe extern "C" fn m_read(ptr: *mut c_void, size: usize, n: usize, con: *mut Rconn) -> usize {
    if size == 0 {
        return 0;
    }
    R_ReadConnection(con as *mut c_void, ptr, size * n) / size
}

unsafe extern "C" fn m_write(ptr: *const c_void, size: usize, n: usize, con: *mut Rconn) -> usize {
    if size == 0 {
        return 0;
    }
    R_WriteConnection(con as *mut c_void, ptr, size * n) / size
}

// R: connections.c:getConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getConnection(n: c_int) -> *mut Rconn {
    let by = BY_NUMBER.get_or_insert_with(HashMap::new);
    let con = match by.get(&n) {
        Some(&p) => p as *mut Rconn,
        None => {
            // the host validates the number ("invalid connection")
            Rf_protect(op("connInfo\0", &[Rf_ScalarInteger(n)]));
            Rf_unprotect(1);
            let mut c: Box<Rconn> = Box::new(std::mem::zeroed());
            c.open = Some(m_open);
            c.close = Some(m_close);
            c.destroy = Some(m_destroy);
            c.vfprintf = Some(m_vfprintf);
            c.fgetc = Some(m_fgetc);
            c.fgetc_internal = Some(m_fgetc);
            c.seek = Some(m_seek);
            c.truncate = Some(m_truncate);
            c.fflush = Some(m_fflush);
            c.read = Some(m_read);
            c.write = Some(m_write);
            c.save = -1000;
            c.save2 = -1000;
            let p = Box::into_raw(c);
            by.insert(n, p as usize);
            CONNS
                .get_or_insert_with(HashMap::new)
                .insert(p as usize, Entry { n, strings: (CString::default(), CString::default()) });
            p
        }
    };
    refresh(con);
    con
}

// R: connections.c:R_GetConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_GetConnection(s_conn: Sexp) -> *mut c_void {
    let n = *INTEGER(op("connectionNumber\0", &[s_conn]));
    getConnection(n) as *mut c_void
}

// R: connections.c:R_WriteConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_WriteConnection(con: *mut c_void, buf: *const c_void, n: usize) -> usize {
    let num = number_of(con);
    let raw = Rf_protect(crate::state::core().alloc_vector(RAWSXP, n));
    if n > 0 {
        std::ptr::copy_nonoverlapping(buf as *const u8, raw.std_data(), n);
    }
    op("writeConnection\0", &[Rf_ScalarInteger(num), raw]);
    Rf_unprotect(1);
    n
}

// R: connections.c:R_ReadConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ReadConnection(con: *mut c_void, buf: *mut c_void, n: usize) -> usize {
    let num = number_of(con);
    let r = op("readConnection\0", &[Rf_ScalarInteger(num), Rf_ScalarInteger(n.min(i32::MAX as usize) as c_int)]);
    let got = xlength(r) as usize;
    if got > 0 {
        std::ptr::copy_nonoverlapping(r.std_data(), buf as *mut u8, got);
    }
    got
}

// R: connections.c:Rconn_fgetc
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rconn_fgetc(con: *mut c_void) -> c_int {
    *INTEGER(op("connGetc\0", &[Rf_ScalarInteger(number_of(con))]))
}
