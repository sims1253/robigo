//! R_ext/Connections.h: the C API to R connections, over the host's
//! connection table. C code gets a GNU R `struct Rconn` per connection
//! (the layout is public, `R_CONNECTIONS_VERSION` 1): its fields mirror
//! the host's connection and are refreshed whenever C asks for it or
//! calls `open`/`close`; its methods call the host.

use crate::errors::{Res, error_str, ret, throw};
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

pub(crate) struct Entry {
    n: c_int,
    /// the strings `class` and `description` point into
    strings: (CString, CString),
    /// The package's own struct (R_new_custom_connection), not a mirror:
    /// its fields and methods are the connection's (custom_conn.rs).
    native: bool,
}

/// The connections C sees, by address (the isolate's `conns`).
fn conns() -> &'static RefCell<HashMap<usize, Entry>> {
    // SAFETY: a shared reference to this thread's isolate's table. A
    // `RefCell` is not `Sync`, so it cannot leave the thread; a second
    // thread initializing the API aborts at the `OWNER` claim
    // (`state::init_state`); the isolate is never freed, so `'static`
    // holds. Callers still hold no borrow across a call into package or R
    // code (`state::Tables`).
    unsafe { &crate::state::tables().conns }
}

/// The connections C sees, by number (the isolate's `conn_by_number`).
fn by_number() -> &'static RefCell<HashMap<c_int, usize>> {
    // SAFETY: as in `conns`.
    unsafe { &crate::state::tables().conn_by_number }
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

#[inline(always)]
pub unsafe fn number_of(con: *mut c_void) -> Res<c_int> {
    let key = con as usize;
    if key != 0 && LAST_KEY.load(Ordering::Relaxed) == key {
        return Ok(LAST_NUMBER.load(Ordering::Relaxed));
    }
    number_of_slow(key)
}

/// [`number_of`] of a connection other than the last one.
#[cold]
#[inline(never)]
unsafe fn number_of_slow(key: usize) -> Res<c_int> {
    let known = conns().borrow().get(&key).map(|e| e.n);
    match known {
        Some(n) => {
            LAST_NUMBER.store(n, Ordering::Relaxed);
            LAST_KEY.store(key, Ordering::Relaxed);
            Ok(n)
        }
        None => Err(error_str("invalid connection")),
    }
}

/// Copy the host connection's fields into its `struct Rconn`.
unsafe fn refresh(con: *mut Rconn) -> Res<()> {
    let n = number_of(con as *mut c_void)?;
    let info = protect(op("connInfo\0", &[Rf_ScalarInteger(n)])?)?;
    let s = |k: isize| -> Res<CString> {
        let c = string_elt(vector_elt(info, k)?, 0)?;
        Ok(CString::new(c.char_bytes().to_vec()).unwrap_or_default())
    };
    let (class, desc, mode) = (s(0)?, s(1)?, s(2)?);
    let flags = std::slice::from_raw_parts(logical(vector_elt(info, 3)?)?, 7);
    unprotect(1)?;
    let c = &mut *con;
    c.mode = [0; 5];
    for (k, &b) in mode.as_bytes().iter().take(4).enumerate() {
        c.mode[k] = b as c_char;
    }
    [c.text, c.isopen, c.incomplete, c.canread, c.canwrite, c.canseek, c.blocking] =
        [flags[0], flags[1], flags[2], flags[3], flags[4], flags[5], flags[6]];
    if let Some(e) = conns().borrow_mut().get_mut(&(con as usize)) {
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
    Ok(())
}

unsafe extern "C" fn m_open(con: *mut Rconn) -> c_int {
    ret(conn_open(con))
}

unsafe fn conn_open(con: *mut Rconn) -> Res<c_int> {
    let n = number_of(con as *mut c_void)?;
    let ok = *logical(op("connOpen\0", &[Rf_ScalarInteger(n)])?)?;
    refresh(con)?;
    Ok(ok)
}

unsafe extern "C" fn m_close(con: *mut Rconn) {
    ret(conn_close(con))
}

unsafe fn conn_close(con: *mut Rconn) -> Res<()> {
    let n = number_of(con as *mut c_void)?;
    op("connClose\0", &[Rf_ScalarInteger(n)])?;
    refresh(con)
}

unsafe extern "C" fn m_destroy(_: *mut Rconn) {}

unsafe extern "C" {
    fn vasprintf(strp: *mut *mut c_char, fmt: *const c_char, ap: *mut c_void) -> c_int;
    fn robigo_connection_string_arg(fmt: *const c_char, ap: *mut c_void) -> *const c_char;
}

unsafe extern "C" fn m_vfprintf(con: *mut Rconn, fmt: *const c_char, ap: *mut c_void) -> c_int {
    ret(conn_vfprintf(con, fmt, ap))
}

unsafe fn conn_vfprintf(con: *mut Rconn, fmt: *const c_char, ap: *mut c_void) -> Res<c_int> {
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
            t?;
            return Ok(n as c_int);
        }
    }
    let mut s: *mut c_char = std::ptr::null_mut();
    let r = vasprintf(&mut s, fmt, ap);
    if r < 0 {
        return Ok(r);
    }
    let bytes = std::slice::from_raw_parts(s as *const u8, r as usize).to_vec();
    libc::free(s as *mut c_void);
    write_token(con as *mut c_void, bytes.as_ptr() as *const c_void, bytes.len())?;
    Ok(r)
}

unsafe extern "C" fn m_fgetc(con: *mut Rconn) -> c_int {
    ret(conn_fgetc(con as *mut c_void))
}

unsafe extern "C" fn m_seek(_: *mut Rconn, _: f64, _: c_int, _: c_int) -> f64 {
    throw(error_str("robigo: seeking a connection from C is not implemented yet"))
}

unsafe extern "C" fn m_truncate(_: *mut Rconn) {
    throw(error_str("robigo: truncating a connection from C is not implemented yet"))
}

unsafe extern "C" fn m_fflush(_: *mut Rconn) -> c_int {
    0
}

unsafe extern "C" fn m_read(ptr: *mut c_void, size: usize, n: usize, con: *mut Rconn) -> usize {
    if size == 0 {
        return 0;
    }
    // robigo's own connections only: a package's struct has its own read
    ret(read_host_connection(con as *mut c_void, ptr, size * n)) / size
}

unsafe extern "C" fn m_write(ptr: *const c_void, size: usize, n: usize, con: *mut Rconn) -> usize {
    if size == 0 {
        return 0;
    }
    ret(write_token(con as *mut c_void, ptr, size * n));
    n
}

// R: connections.c:getConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getConnection(n: c_int) -> *mut Rconn {
    ret(get_connection(n))
}

/// getConnection.
pub unsafe fn get_connection(n: c_int) -> Res<*mut Rconn> {
    let known = by_number().borrow().get(&n).copied();
    let con = match known {
        Some(p) => p as *mut Rconn,
        None => {
            // the host validates the number ("invalid connection")
            protect(op("connInfo\0", &[Rf_ScalarInteger(n)])?)?;
            unprotect(1)?;
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
            by_number().borrow_mut().insert(n, p as usize);
            let strings = (CString::default(), CString::default());
            conns().borrow_mut().insert(p as usize, Entry { n, strings, native: false });
            p
        }
    };
    if !is_native(con) {
        refresh(con)?;
    }
    Ok(con)
}

/// Whether `con` is a package's own struct (R_new_custom_connection).
/// Free while none exists: write.table asks for every cell it writes.
#[inline]
pub(crate) fn is_native(con: *mut Rconn) -> bool {
    any_native() && conns().borrow().get(&(con as usize)).is_some_and(|e| e.native)
}

/// Connection `n` is the package's struct `con` from now on: getConnection
/// returns it.
pub(crate) fn register_native(con: *mut Rconn, n: c_int) {
    let strings = (CString::default(), CString::default());
    conns().borrow_mut().insert(con as usize, Entry { n, strings, native: true });
    by_number().borrow_mut().insert(n, con as usize);
    NATIVES.fetch_add(1, Ordering::Relaxed);
}

/// The struct `con` is gone (destroyed and freed).
pub(crate) fn unregister_native(con: *mut Rconn) {
    let key = con as usize;
    let removed = conns().borrow_mut().remove(&key);
    if let Some(e) = removed {
        if e.native {
            NATIVES.fetch_sub(1, Ordering::Relaxed);
        }
        let mut by = by_number().borrow_mut();
        if by.get(&e.n) == Some(&key) {
            by.remove(&e.n);
        }
    }
    if LAST_KEY.load(Ordering::Relaxed) == key {
        LAST_KEY.store(0, Ordering::Relaxed);
    }
}

/// The package structs and their connection numbers.
pub(crate) fn native_structs() -> Vec<(*mut Rconn, c_int)> {
    conns().borrow().iter().filter(|(_, e)| e.native).map(|(&k, e)| (k as *mut Rconn, e.n)).collect()
}

/// `f` on each package struct and its connection number, without a copy
/// of the table; `f` must not register or remove structs.
pub(crate) fn for_each_native(mut f: impl FnMut(*mut Rconn, c_int)) {
    for (&k, e) in conns().borrow().iter() {
        if e.native {
            f(k as *mut Rconn, e.n);
        }
    }
}

/// The package struct of connection `n`, if it is one.
pub(crate) fn native_of(n: c_int) -> Option<*mut Rconn> {
    let p = by_number().borrow().get(&n).copied()? as *mut Rconn;
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
    ret(op("connectionNumber\0", &[s_conn]).and_then(|r| get_connection(*integer(r)?))) as *mut c_void
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
            throw(error_str("connection is not open"));
        }
        if (*c).canwrite == 0 {
            throw(error_str("cannot write to this connection"));
        }
        // the package's method, called from this wrapper's frame
        return match (*c).write {
            Some(write) => write(buf, 1, n, c),
            None => throw(error_str("robigo: the connection has no write method")),
        };
    }
    ret(write_token(con, buf, n));
    n
}

/// Write to one of robigo's own connections.
#[inline(always)]
unsafe fn write_token(con: *mut c_void, buf: *const c_void, n: usize) -> Res<()> {
    let num = number_of(con)?;
    let bytes = if n == 0 { &[] } else { std::slice::from_raw_parts(buf as *const u8, n) };
    crate::errors::flow(crate::bridge::write_connection_bytes(num, bytes))
}

// R: connections.c:R_ReadConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ReadConnection(con: *mut c_void, buf: *mut c_void, n: usize) -> usize {
    if is_native(con as *mut Rconn) {
        let c = con as *mut Rconn;
        if (*c).isopen == 0 {
            throw(error_str("connection is not open"));
        }
        if (*c).canread == 0 {
            throw(error_str("cannot read from this connection"));
        }
        // the package's method, called from this wrapper's frame
        return match (*c).read {
            Some(read) => read(buf, 1, n, c),
            None => throw(error_str("robigo: the connection has no read method")),
        };
    }
    ret(read_host_connection(con, buf, n))
}

/// Read from one of robigo's own connections.
unsafe fn read_host_connection(con: *mut c_void, buf: *mut c_void, n: usize) -> Res<usize> {
    let num = number_of(con)?;
    let r = op("readConnection\0", &[Rf_ScalarInteger(num), Rf_ScalarInteger(n.min(i32::MAX as usize) as c_int)])?;
    let got = xlength(r)? as usize;
    if got > 0 {
        std::ptr::copy_nonoverlapping(r.std_data(), buf as *mut u8, got);
    }
    Ok(got)
}

// R: connections.c:Rconn_fgetc
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rconn_fgetc(con: *mut c_void) -> c_int {
    ret(conn_fgetc(con))
}

unsafe fn conn_fgetc(con: *mut c_void) -> Res<c_int> {
    Ok(*integer(op("connGetc\0", &[Rf_ScalarInteger(number_of(con)?)])?)?)
}
