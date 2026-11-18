//! connections.c's C view (the A5 note's decision C): `getConnection`,
//! `R_GetConnection` and `Rconn_fgetc`, and the methods a connection's
//! `struct Rconn` points at (`con->open(con)`, ...), which call
//! robigo-rt's connections. The struct is the connection's own state
//! (`robigo_rt::rconn`), so what C writes there is what R sees.

use crate::entry;
use crate::errors::not_yet;
use robigo_rt::rconn::{CMethods, RconnC};
use robigo_rtcore::{Cx, Sexp};
use std::ffi::{c_char, c_int, c_void};

/// The methods every connection's `struct Rconn` points at, installed in
/// `cx`'s isolate.
pub(crate) fn install(cx: &mut Cx) {
    robigo_rt::connections::set_c_methods(
        cx,
        CMethods {
            open: robigo_rtapi_conn_open,
            close: robigo_rtapi_conn_close,
            destroy: robigo_rtapi_conn_destroy,
            vfprintf: robigo_rtapi_conn_vfprintf,
            fgetc: robigo_rtapi_conn_fgetc,
            fgetc_internal: robigo_rtapi_conn_fgetc_internal,
            seek: robigo_rtapi_conn_seek,
            truncate: robigo_rtapi_conn_truncate,
            fflush: robigo_rtapi_conn_fflush,
            read: robigo_rtapi_conn_read,
            write: robigo_rtapi_conn_write,
        },
    );
}

entry! {
    /// `getConnection(n)`: connection `n`'s `struct Rconn` ("invalid
    /// connection" for none).
    // R: connections.c:getConnection
    [1] fn getConnection(n: c_int) -> *mut c_void => |cx, _s| {
        Ok(robigo_rt::connections::get_connection(cx, n)?.cast())
    }

    /// `R_GetConnection(sConn)`: the connection object's `struct Rconn`.
    // R: connections.c:R_GetConnection
    [1] fn R_GetConnection(sconn: Sexp) -> *mut c_void => |cx, _s| {
        if !robigo_rtbuiltins::attrib::inherits(sconn.bind(cx), b"connection") {
            return Err(cx.error("invalid connection"));
        }
        let n = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, sconn)?;
        Ok(robigo_rt::connections::get_connection(cx, n)?.cast())
    }

    /// `Rconn_fgetc(con)`.
    // R: connections.c:Rconn_fgetc
    [1] fn Rconn_fgetc(con: *mut c_void) -> c_int => |cx, _s| {
        robigo_rt::connections::rconn_fgetc(cx, con.cast())
    }

    /// `con->open(con)`.
    [1] fn robigo_rtapi_conn_open(con: *mut RconnC) -> c_int => |cx, _s| {
        Ok(robigo_rt::connections::c_open(cx, con)? as c_int)
    }

    /// `con->close(con)`.
    [1] fn robigo_rtapi_conn_close(con: *mut RconnC) -> () => |cx, _s| {
        robigo_rt::connections::c_close(cx, con)
    }

    /// `con->destroy(con)`: GNU R's `con_destroy` frees the private part;
    /// C has no reason to call it.
    [1] fn robigo_rtapi_conn_destroy(con: *mut RconnC) -> () => |cx, _s| {
        let _ = con;
        Err(not_yet(cx, "a connection's destroy method called from C"))
    }

    /// `con->vfprintf(con, format, ap)`.
    [3] fn robigo_rtapi_conn_vfprintf(con: *mut RconnC, format: *const c_char, ap: *mut c_void) -> c_int => |cx, _s| {
        let _ = (con, format, ap);
        Err(not_yet(cx, "a connection's vfprintf method called from C"))
    }

    /// `con->fgetc(con)` (`dummy_fgetc`).
    [1] fn robigo_rtapi_conn_fgetc(con: *mut RconnC) -> c_int => |cx, _s| {
        robigo_rt::connections::c_fgetc(cx, con)
    }

    /// `con->fgetc_internal(con)`.
    [1] fn robigo_rtapi_conn_fgetc_internal(con: *mut RconnC) -> c_int => |cx, _s| {
        robigo_rt::connections::c_fgetc_internal(cx, con)
    }

    /// `con->seek(con, where, origin, rw)`: robigo-rt has no seek yet.
    [3] fn robigo_rtapi_conn_seek(con: *mut RconnC, at: f64, origin: c_int, rw: c_int) -> f64 => |cx, _s| {
        let _ = (con, at, origin, rw);
        Err(not_yet(cx, "a connection's seek method called from C"))
    }

    /// `con->truncate(con)`.
    [1] fn robigo_rtapi_conn_truncate(con: *mut RconnC) -> () => |cx, _s| {
        let _ = con;
        Err(not_yet(cx, "a connection's truncate method called from C"))
    }

    /// `con->fflush(con)`.
    [1] fn robigo_rtapi_conn_fflush(con: *mut RconnC) -> c_int => |cx, _s| {
        robigo_rt::connections::c_fflush(cx, con)
    }

    /// `con->read(ptr, size, nitems, con)`.
    [4] fn robigo_rtapi_conn_read(ptr: *mut c_void, size: usize, nitems: usize, con: *mut RconnC) -> usize => |cx, _s| {
        let n = size.saturating_mul(nitems);
        if n == 0 {
            return Ok(0);
        }
        // SAFETY: C's contract: room for nitems items of size bytes.
        let buf = unsafe { std::slice::from_raw_parts_mut(ptr.cast::<u8>(), n) };
        robigo_rt::connections::c_read(cx, buf, size, con)
    }

    /// `con->write(ptr, size, nitems, con)`.
    [4] fn robigo_rtapi_conn_write(ptr: *const c_void, size: usize, nitems: usize, con: *mut RconnC) -> usize => |cx, _s| {
        let n = size.saturating_mul(nitems);
        if n == 0 {
            return Ok(0);
        }
        // SAFETY: C's contract: nitems items of size bytes.
        let buf = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), n) };
        robigo_rt::connections::c_write(cx, buf, size, con)
    }
}
