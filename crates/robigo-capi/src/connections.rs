//! R_ext/Connections.h: the C API to R connections, over the host's
//! connection table. An `Rconnection` handed to C is an opaque, zeroed
//! block standing for a connection number (robigo's connections are not
//! GNU R's `struct Rconn`; code that reads its fields is not supported).

use crate::host::op;
use crate::memory::*;
use robigo_core::Sexp;
use robigo_core::types::*;
use std::collections::HashMap;
use std::ffi::{c_int, c_void};

static mut CONNS: Option<HashMap<usize, c_int>> = None;
static mut BY_NUMBER: Option<HashMap<c_int, usize>> = None;

unsafe fn number_of(con: *mut c_void) -> c_int {
    match CONNS.as_ref().and_then(|m| m.get(&(con as usize))) {
        Some(&n) => n,
        None => crate::errors::error_str("invalid connection"),
    }
}

// R: connections.c:R_GetConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_GetConnection(s_conn: Sexp) -> *mut c_void {
    let n = *INTEGER(op("connectionNumber\0", &[s_conn]));
    let by = BY_NUMBER.get_or_insert_with(HashMap::new);
    if let Some(&p) = by.get(&n) {
        return p as *mut c_void;
    }
    // a block the size of GNU R's struct Rconn, zeroed
    let p = libc::calloc(1, 512);
    by.insert(n, p as usize);
    CONNS.get_or_insert_with(HashMap::new).insert(p as usize, n);
    p
}

// R: connections.c:R_WriteConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_WriteConnection(con: *mut c_void, buf: *const c_void, n: usize) -> usize {
    let num = number_of(con);
    let raw = Rf_protect(crate::state::core().alloc_vector(RAWSXP, n));
    if n > 0 {
        std::ptr::copy_nonoverlapping(buf as *const u8, raw.std_data(), n);
    }
    op("writeConnection\0", &[crate::list::Rf_ScalarInteger(num), raw]);
    Rf_unprotect(1);
    n
}

// R: connections.c:R_ReadConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ReadConnection(con: *mut c_void, buf: *mut c_void, n: usize) -> usize {
    let num = number_of(con);
    let r = op(
        "readConnection\0",
        &[crate::list::Rf_ScalarInteger(num), crate::list::Rf_ScalarInteger(n.min(i32::MAX as usize) as c_int)],
    );
    let got = xlength(r) as usize;
    if got > 0 {
        std::ptr::copy_nonoverlapping(r.std_data(), buf as *mut u8, got);
    }
    got
}

// R: connections.c:Rconn_fgetc
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rconn_fgetc(con: *mut c_void) -> c_int {
    *INTEGER(op("connGetc\0", &[crate::list::Rf_ScalarInteger(number_of(con))]))
}
