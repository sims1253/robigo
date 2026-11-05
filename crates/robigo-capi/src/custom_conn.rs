//! Connections C code makes (R_ext/Connections.h `R_new_custom_connection`):
//! the package fills in GNU R's `struct Rconn`, which this crate allocates
//! (main/robigo_connections.c, connections.c's code) and owns until the
//! connection is destroyed. The runtime's connection (`ConnKind::Native`)
//! calls the struct's function pointers through the `NativeBridge` methods
//! here: each copies the runtime's fields into the struct, calls the method
//! in a context (an `Rf_error` from it becomes the runtime's error), and
//! copies the fields back, also on the error path. The fields are synced
//! wherever control passes between C and R code, so that while C runs the
//! struct is the connection's state, and while R runs the runtime's copy
//! is: into the structs when a native call starts and when a callback
//! returns to C (`Rf_eval`, a warning's handlers, a host operation), back
//! when the native call returns and when C calls back into R. The package
//! reads and writes its fields directly (curl sets them after
//! `R_new_custom_connection`).

use crate::connections::{Rconn, for_each_native, native_of, register_native, unregister_native};
use crate::host::op;
use crate::list::Rf_ScalarInteger;
use crate::memory::*;
use crate::state::*;
use robigo_core::Sexp;
use robigo_runtime::builtins::io::Connection;
use robigo_runtime::error::{Flow, RResult};
use robigo_runtime::interp::Interp;
use robigo_runtime::value::Value;
use std::ffi::{CStr, CString, c_char, c_int, c_void};

unsafe extern "C" {
    fn robigo_conn_vfprintf(con: *mut Rconn, s: *const c_char) -> c_int;
    fn robigo_destroy_connection(con: *mut Rconn);
}

// ----------------------------------------------------------------------
// The hooks main/robigo_connections.c calls where connections.c uses
// its table

/// R: connections.c:NextConnection
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_next_connection() -> c_int {
    *INTEGER(op("nextConnection\0", &[]))
}

/// R: connections.c:R_new_custom_connection's `Connections[ncon] = new`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_install_connection(ncon: c_int, con: *mut Rconn) {
    let n = Rf_protect(Rf_ScalarInteger(ncon));
    let p = Rf_protect(crate::extptr::R_MakeExternalPtr(con as *mut c_void, R_NilValue, R_NilValue));
    op("installNativeConnection\0", &[n, p]);
    Rf_unprotect(2);
    register_native(con, ncon);
}

/// conFinalizer's search of `Connections[]`: the connection whose
/// `con->id` is `id`, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_connection_with_id(id: *mut c_void) -> c_int {
    let p = Rf_protect(crate::extptr::R_MakeExternalPtr(id, R_NilValue, R_NilValue));
    let n = *INTEGER(op("connectionWithId\0", &[p]));
    Rf_unprotect(1);
    n
}

/// R: connections.c:con_destroy
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_con_destroy(ncon: c_int) {
    op("conDestroy\0", &[Rf_ScalarInteger(ncon)]);
}

/// init_con's `current_id`, shared with the runtime's connections.
#[unsafe(no_mangle)]
pub extern "C" fn robigo_next_connection_id() -> *mut c_void {
    robigo_runtime::builtins::io::next_connection_id() as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_forget_conn_id(ptr: Sexp) {
    crate::connections::forget_conn_id(ptr)
}

// ----------------------------------------------------------------------
// Fields

fn flag(b: bool) -> c_int {
    b as c_int
}

/// The runtime's fields of the connection into its struct.
unsafe fn push(con: *mut Rconn, c: &Connection) {
    let s = &mut *con;
    s.mode = [0; 5];
    for (k, &b) in c.mode.as_bytes().iter().take(4).enumerate() {
        s.mode[k] = b as c_char;
    }
    s.text = flag(c.text);
    s.isopen = flag(c.isopen);
    s.incomplete = flag(c.incomplete);
    s.canread = flag(c.canread);
    s.canwrite = flag(c.canwrite);
    s.canseek = flag(c.canseek);
    s.blocking = flag(c.blocking);
    s.status = c.status;
}

/// The struct's fields into the runtime's connection.
unsafe fn pull(con: *const Rconn, c: &mut Connection) {
    let s = &*con;
    let len = s.mode.iter().position(|&b| b == 0).unwrap_or(s.mode.len());
    let mode = std::slice::from_raw_parts(s.mode.as_ptr() as *const u8, len);
    if c.mode.as_bytes() != mode {
        c.mode = String::from_utf8_lossy(mode).into_owned();
    }
    c.text = s.text != 0;
    c.isopen = s.isopen != 0;
    c.incomplete = s.incomplete != 0;
    c.canread = s.canread != 0;
    c.canwrite = s.canwrite != 0;
    c.canseek = s.canseek != 0;
    c.blocking = s.blocking != 0;
    c.status = s.status;
}

/// Connection `n`, if it is still the native one over `con`.
fn same(i: &mut Interp, n: c_int, con: *mut Rconn, id: usize) -> Option<&mut Connection> {
    let c = i.conns.get(n).ok()?;
    (c.id == id && robigo_runtime::builtins::io::native_struct_of(c) == Some(con as usize)).then_some(c)
}

/// Before a native call: the runtime's fields into every package struct.
pub(crate) unsafe fn sync_to_c(i: &mut Interp) {
    if !crate::connections::any_native() {
        return;
    }
    for_each_native(|con, n| {
        if let Some(c) = same(i, n, con, (*con).id as usize) {
            push(con, c);
        }
    });
}

/// After a native call: what the package wrote into its structs.
pub(crate) unsafe fn sync_from_c(i: &mut Interp) {
    if !crate::connections::any_native() {
        return;
    }
    for_each_native(|con, n| {
        if let Some(c) = same(i, n, con, (*con).id as usize) {
            pull(con, c);
        }
    });
}

/// C calls back into R code: what it wrote into its structs.
pub(crate) unsafe fn c_to_r() {
    if crate::connections::any_native() {
        crate::bridge::with_interp(|i| sync_from_c(i));
    }
}

/// The callback returns to C: what R code changed, into the structs.
pub(crate) unsafe fn r_to_c() {
    if crate::connections::any_native() {
        crate::bridge::with_interp(|i| sync_to_c(i));
    }
}

/// `installNativeConnection`: the runtime's half of the new connection,
/// with the struct's strings, id and fields (init_con's).
pub(crate) unsafe fn install(i: &mut Interp, n: c_int, con: *mut Rconn) {
    let s = &*con;
    let class = CStr::from_ptr(s.class).to_string_lossy().into_owned();
    let description = CStr::from_ptr(s.description).to_string_lossy().into_owned();
    robigo_runtime::builtins::io::install_native_connection(i, n, &class, &description, con as usize, s.id as usize);
    if let Ok(c) = i.conns.get(n) {
        pull(con, c);
    }
}

// ----------------------------------------------------------------------
// Methods

#[derive(Clone, Copy)]
enum Method {
    Open,
    Close,
    Destroy,
    Fgetc,
    Read,
    Write,
    Vfprintf,
    Seek,
    Truncate,
    Fflush,
}

/// One method call, for the trampoline.
#[repr(C)]
struct Call {
    con: *mut Rconn,
    method: Method,
    /// `Method::Read`'s target
    buf: *mut c_void,
    /// `Method::Write`'s source
    cbuf: *const c_void,
    len: usize,
    text: *const c_char,
    r#where: f64,
    origin: c_int,
    rw: c_int,
    int_ret: c_int,
    size_ret: usize,
    real_ret: f64,
}

impl Call {
    fn new(method: Method) -> Call {
        Call {
            con: std::ptr::null_mut(),
            method,
            buf: std::ptr::null_mut(),
            cbuf: std::ptr::null(),
            len: 0,
            text: std::ptr::null(),
            r#where: 0.0,
            origin: 0,
            rw: 0,
            int_ret: 0,
            size_ret: 0,
            real_ret: 0.0,
        }
    }
}

fn missing(what: &str) -> ! {
    // SAFETY: called inside the trampoline's context.
    unsafe { crate::errors::error_str(&format!("robigo: the connection has no {what} method")) }
}

/// The method call (in a context: it may jump).
unsafe extern "C" fn call_method(data: *mut c_void) {
    let c = &mut *(data as *mut Call);
    let con = c.con;
    match c.method {
        Method::Open => c.int_ret = ((*con).open.unwrap_or_else(|| missing("open")))(con),
        Method::Close => ((*con).close.unwrap_or_else(|| missing("close")))(con),
        Method::Destroy => robigo_destroy_connection(con),
        Method::Fgetc => c.int_ret = ((*con).fgetc.unwrap_or_else(|| missing("fgetc")))(con),
        Method::Read => c.size_ret = ((*con).read.unwrap_or_else(|| missing("read")))(c.buf, 1, c.len, con),
        Method::Write => c.size_ret = ((*con).write.unwrap_or_else(|| missing("write")))(c.cbuf, 1, c.len, con),
        Method::Vfprintf => {
            if (*con).vfprintf.is_none() {
                missing("vfprintf");
            }
            c.int_ret = robigo_conn_vfprintf(con, c.text)
        }
        Method::Seek => c.real_ret = ((*con).seek.unwrap_or_else(|| missing("seek")))(con, c.r#where, c.origin, c.rw),
        Method::Truncate => ((*con).truncate.unwrap_or_else(|| missing("truncate")))(con),
        Method::Fflush => c.int_ret = ((*con).fflush.unwrap_or_else(|| missing("fflush")))(con),
    }
}

/// Call a method of native connection `n`: fields in, the call with no
/// borrow of the table held, then fields back from the struct if `n` is
/// still that connection (the method may have run R code that closed it).
unsafe fn call(i: &mut Interp, n: c_int, mut c: Call) -> RResult<Call> {
    let (con, id) = {
        let conn = i.conns.get(n)?;
        let Some(con) = robigo_runtime::builtins::io::native_struct_of(conn) else {
            return Err(Flow::error("invalid connection"));
        };
        let con = con as *mut Rconn;
        push(con, conn);
        (con, conn.id)
    };
    c.con = con;
    let destroy = matches!(c.method, Method::Destroy);
    if destroy {
        unregister_native(con);
    }
    let r = crate::errors::try_void_ptr(call_method, &mut c as *mut Call as *mut c_void);
    if !destroy {
        if let Some(conn) = same(i, n, con, id) {
            pull(con, conn);
        }
    }
    match r {
        Ok(()) => Ok(c),
        Err(t) => Err(crate::bridge::take_flow(t)),
    }
}

pub(crate) unsafe fn open(i: &mut Interp, n: c_int) -> RResult<bool> {
    Ok(call(i, n, Call::new(Method::Open))?.int_ret != 0)
}

pub(crate) unsafe fn close(i: &mut Interp, n: c_int) -> RResult<()> {
    call(i, n, Call::new(Method::Close)).map(|_| ())
}

pub(crate) unsafe fn destroy(i: &mut Interp, n: c_int) -> RResult<()> {
    call(i, n, Call::new(Method::Destroy)).map(|_| ())
}

pub(crate) unsafe fn fgetc(i: &mut Interp, n: c_int) -> RResult<c_int> {
    Ok(call(i, n, Call::new(Method::Fgetc))?.int_ret)
}

pub(crate) unsafe fn read(i: &mut Interp, n: c_int, buf: &mut [u8]) -> RResult<usize> {
    let mut c = Call::new(Method::Read);
    c.buf = buf.as_mut_ptr() as *mut c_void;
    c.len = buf.len();
    // A read method returns size_t; a negative one is its error, which
    // readBin, readOneString and buff_fill raise
    // R: connections.c:do_readbin (`if (m0 < 0) error(...)`)
    let got = call(i, n, c)?.size_ret as isize;
    if got < 0 {
        return Err(Flow::error("error reading from the connection"));
    }
    Ok((got as usize).min(buf.len()))
}

pub(crate) unsafe fn write(i: &mut Interp, n: c_int, bytes: &[u8]) -> RResult<usize> {
    let mut c = Call::new(Method::Write);
    c.cbuf = bytes.as_ptr() as *const c_void;
    c.len = bytes.len();
    Ok(call(i, n, c)?.size_ret)
}

pub(crate) unsafe fn vfprintf(i: &mut Interp, n: c_int, text: &str) -> RResult<()> {
    // the formatted text is a C string: it ends at a NUL, as R's does
    let end = text.bytes().position(|b| b == 0).unwrap_or(text.len());
    let s = CString::new(&text.as_bytes()[..end]).unwrap_or_default();
    let mut c = Call::new(Method::Vfprintf);
    c.text = s.as_ptr();
    call(i, n, c).map(|_| ())
}

pub(crate) unsafe fn seek(i: &mut Interp, n: c_int, r#where: f64, origin: c_int, rw: c_int) -> RResult<f64> {
    let mut c = Call::new(Method::Seek);
    c.r#where = r#where;
    c.origin = origin;
    c.rw = rw;
    Ok(call(i, n, c)?.real_ret)
}

pub(crate) unsafe fn truncate(i: &mut Interp, n: c_int) -> RResult<()> {
    call(i, n, Call::new(Method::Truncate)).map(|_| ())
}

pub(crate) unsafe fn fflush(i: &mut Interp, n: c_int) -> RResult<c_int> {
    Ok(call(i, n, Call::new(Method::Fflush))?.int_ret)
}

/// The `conn_id` of native connection `n`: its `con->ex_ptr`, or NULL
/// once that died.
pub(crate) unsafe fn conn_id_value(n: c_int) -> Value {
    match native_of(n) {
        Some(con) if !(*con).ex_ptr.is_null() => crate::bridge::wrap_core(Sexp((*con).ex_ptr as *mut u8)),
        _ => Value::Null,
    }
}
