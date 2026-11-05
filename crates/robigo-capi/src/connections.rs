//! R_ext/Connections.h: the C API to R connections, over the host's
//! connection table. C code gets a GNU R `struct Rconn` per connection
//! (the layout is public, `R_CONNECTIONS_VERSION` 1): its fields mirror
//! the host's connection and are refreshed whenever C asks for it or
//! calls `open`/`close`; its methods call the host.

use crate::host::op;
use crate::list::Rf_ScalarInteger;
use crate::memory::*;
use robigo_core::Sexp;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

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
    /// The package's own struct (R_new_custom_connection), not a mirror:
    /// its fields and methods are the connection's (custom_conn.rs).
    native: bool,
}

thread_local! {
    static CONNS: RefCell<HashMap<usize, Entry>> = RefCell::new(HashMap::new());
    static BY_NUMBER: RefCell<HashMap<c_int, usize>> = RefCell::new(HashMap::new());
}

/// The last connection pointer and its number. Connections are never
/// removed and their numbers never change, so this stays valid; native
/// table writers hand every cell to the same connection, so this is read per
/// cell: atomics, which cost nothing there, rather than a thread-local. The API
/// belongs to one thread, so the two parts never disagree.
static LAST_KEY: AtomicUsize = AtomicUsize::new(0);
/// How many package structs exist (R_new_custom_connection), so that a
/// native call without any pays nothing for syncing them.
static NATIVES: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn any_native() -> bool {
    NATIVES.load(Ordering::Relaxed) != 0
}
static LAST_NUMBER: AtomicI32 = AtomicI32::new(0);

#[inline]
pub unsafe fn number_of(con: *mut c_void) -> c_int {
    let key = con as usize;
    if key != 0 && LAST_KEY.load(Ordering::Relaxed) == key {
        return LAST_NUMBER.load(Ordering::Relaxed);
    }
    match CONNS.with(|m| m.borrow().get(&key).map(|e| e.n)) {
        Some(n) => {
            LAST_NUMBER.store(n, Ordering::Relaxed);
            LAST_KEY.store(key, Ordering::Relaxed);
            n
        }
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
    c.mode = [0; 5];
    for (k, &b) in mode.as_bytes().iter().take(4).enumerate() {
        c.mode[k] = b as c_char;
    }
    [c.text, c.isopen, c.incomplete, c.canread, c.canwrite, c.canseek, c.blocking] =
        [flags[0], flags[1], flags[2], flags[3], flags[4], flags[5], flags[6]];
    CONNS.with(|m| {
        if let Some(e) = m.borrow_mut().get_mut(&(con as usize)) {
            // C may have saved these pointers (GNU R's stay put for the
            // connection's life): only a string that changed moves.
            if e.strings.0 != class {
                e.strings.0 = class;
            }
            if e.strings.1 != desc {
                e.strings.1 = desc;
            }
            c.class = e.strings.0.as_ptr() as *mut c_char;
            c.description = e.strings.1.as_ptr() as *mut c_char;
        }
    });
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
            let t = if n <= 256 {
                let mut scratch = std::mem::MaybeUninit::<[u8; 256]>::uninit();
                let dst = scratch.as_mut_ptr().cast::<u8>();
                std::ptr::copy_nonoverlapping(arg.cast::<u8>(), dst, n);
                write_token(con as *mut c_void, dst.cast(), n)
            } else {
                let bytes = std::slice::from_raw_parts(arg.cast::<u8>(), n).to_vec();
                write_token(con as *mut c_void, bytes.as_ptr().cast(), n)
            };
            if t != 0 {
                crate::errors::jump_host(t);
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
    let t = write_token(con as *mut c_void, bytes.as_ptr() as *const c_void, bytes.len());
    drop(bytes);
    if t != 0 {
        crate::errors::jump_host(t);
    }
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
    let con = match BY_NUMBER.with(|by| by.borrow().get(&n).copied()) {
        Some(p) => p as *mut Rconn,
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
            BY_NUMBER.with(|by| by.borrow_mut().insert(n, p as usize));
            let strings = (CString::default(), CString::default());
            CONNS.with(|m| m.borrow_mut().insert(p as usize, Entry { n, strings, native: false }));
            p
        }
    };
    if !is_native(con) {
        refresh(con);
    }
    con
}

/// Whether `con` is a package's own struct (R_new_custom_connection).
/// Free while none exists: write.table asks for every cell it writes.
#[inline]
pub(crate) fn is_native(con: *mut Rconn) -> bool {
    any_native() && CONNS.with(|m| m.borrow().get(&(con as usize)).is_some_and(|e| e.native))
}

/// Connection `n` is the package's struct `con` from now on: getConnection
/// returns it.
pub(crate) fn register_native(con: *mut Rconn, n: c_int) {
    let strings = (CString::default(), CString::default());
    CONNS.with(|m| m.borrow_mut().insert(con as usize, Entry { n, strings, native: true }));
    BY_NUMBER.with(|by| by.borrow_mut().insert(n, con as usize));
    NATIVES.fetch_add(1, Ordering::Relaxed);
}

/// The struct `con` is gone (destroyed and freed).
pub(crate) fn unregister_native(con: *mut Rconn) {
    let key = con as usize;
    if let Some(e) = CONNS.with(|m| m.borrow_mut().remove(&key)) {
        if e.native {
            NATIVES.fetch_sub(1, Ordering::Relaxed);
        }
        BY_NUMBER.with(|by| {
            let mut by = by.borrow_mut();
            if by.get(&e.n) == Some(&key) {
                by.remove(&e.n);
            }
        });
    }
    if LAST_KEY.load(Ordering::Relaxed) == key {
        LAST_KEY.store(0, Ordering::Relaxed);
    }
}

/// The package structs and their connection numbers.
pub(crate) fn native_structs() -> Vec<(*mut Rconn, c_int)> {
    CONNS.with(|m| m.borrow().iter().filter(|(_, e)| e.native).map(|(&k, e)| (k as *mut Rconn, e.n)).collect())
}

/// `f` on each package struct and its connection number, without a copy
/// of the table; `f` must not register or remove structs.
pub(crate) fn for_each_native(mut f: impl FnMut(*mut Rconn, c_int)) {
    CONNS.with(|m| {
        for (&k, e) in m.borrow().iter() {
            if e.native {
                f(k as *mut Rconn, e.n);
            }
        }
    })
}

/// The package struct of connection `n`, if it is one.
pub(crate) fn native_of(n: c_int) -> Option<*mut Rconn> {
    let p = BY_NUMBER.with(|by| by.borrow().get(&n).copied())? as *mut Rconn;
    is_native(p).then_some(p)
}

/// A dying `conn_id`: no struct's `ex_ptr` may point to it any longer.
pub(crate) unsafe fn forget_conn_id(ptr: Sexp) {
    for (con, _) in native_structs() {
        if (*con).ex_ptr == ptr.addr() as *mut c_void {
            (*con).ex_ptr = std::ptr::null_mut();
        }
    }
}

// R: connections.c:R_GetConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_GetConnection(s_conn: Sexp) -> *mut c_void {
    let n = *INTEGER(op("connectionNumber\0", &[s_conn]));
    getConnection(n) as *mut c_void
}

// R: connections.c:R_WriteConnection. The bytes go straight to the host
// connection without wrapping them in a RAWSXP and an integer scalar
// and going through a host operation (R:
// library/utils/src/io.c:writetable hands each cell, separator and
// newline to Rconn_printf; m_vfprintf already copied them out of the
// native formatter's static buffer, which con_write_bytes' own
// allocations could otherwise overwrite via finalizers).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_WriteConnection(con: *mut c_void, buf: *const c_void, n: usize) -> usize {
    if is_native(con as *mut Rconn) {
        let c = con as *mut Rconn;
        if (*c).isopen == 0 {
            crate::errors::error_str("connection is not open");
        }
        if (*c).canwrite == 0 {
            crate::errors::error_str("cannot write to this connection");
        }
        return match (*c).write {
            Some(write) => write(buf, 1, n, c),
            None => crate::errors::error_str("robigo: the connection has no write method"),
        };
    }
    let t = write_token(con, buf, n);
    if t != 0 {
        crate::errors::jump_host(t);
    }
    n
}

/// Write to a connection; a nonzero token is the jump the caller makes
/// once it dropped what it owns.
#[inline(always)]
unsafe fn write_token(con: *mut c_void, buf: *const c_void, n: usize) -> usize {
    let num = number_of(con);
    let bytes = if n == 0 { &[] } else { std::slice::from_raw_parts(buf as *const u8, n) };
    crate::bridge::write_connection_bytes(num, bytes)
}

// R: connections.c:R_ReadConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ReadConnection(con: *mut c_void, buf: *mut c_void, n: usize) -> usize {
    if is_native(con as *mut Rconn) {
        let c = con as *mut Rconn;
        if (*c).isopen == 0 {
            crate::errors::error_str("connection is not open");
        }
        if (*c).canread == 0 {
            crate::errors::error_str("cannot read from this connection");
        }
        return match (*c).read {
            Some(read) => read(buf, 1, n, c),
            None => crate::errors::error_str("robigo: the connection has no read method"),
        };
    }
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
