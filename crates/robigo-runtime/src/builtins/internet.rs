//! `download.file(method = "libcurl")`: GNU R's src/modules/internet/
//! libcurl.c `in_do_curlDownload`, over the system's libcurl, which is
//! opened with `dlopen` on first use (robigo does not link against it).

#![allow(unsafe_op_in_unsafe_fn)]

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_void};

use super::{arg, internal};
use crate::coerce::{as_integer, as_logical};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static DEFS: &[Builtin] = &[internal("curlDownload", do_curldownload, 6)];

type Curl = c_void;
type CurlM = c_void;

// curl.h (values checked against the system header, docs/ledger/core.md)
const CURLOPT_URL: c_int = 10002;
const CURLOPT_FAILONERROR: c_int = 45;
const CURLOPT_PIPEWAIT: c_int = 237;
const CURLOPT_CAINFO: c_int = 10065;
const CURLOPT_USERAGENT: c_int = 10018;
const CURLOPT_CONNECTTIMEOUT_MS: c_int = 156;
const CURLOPT_TIMEOUT_MS: c_int = 155;
const CURLOPT_FOLLOWLOCATION: c_int = 52;
const CURLOPT_MAXREDIRS: c_int = 68;
const CURLOPT_VERBOSE: c_int = 41;
const CURLOPT_COOKIEFILE: c_int = 10031;
const CURLOPT_TCP_KEEPALIVE: c_int = 213;
const CURLOPT_HTTPHEADER: c_int = 10023;
const CURLOPT_WRITEDATA: c_int = 10001;
const CURLOPT_PRIVATE: c_int = 10103;
const CURLOPT_NOPROGRESS: c_int = 43;
const CURLOPT_XFERINFOFUNCTION: c_int = 20219;
const CURLOPT_XFERINFODATA: c_int = 10057;
const CURLOPT_TIMEOUT: c_int = 13;
const CURLOPT_PREREQFUNCTION: c_int = 20312;
const CURLOPT_PREREQDATA: c_int = 10313;
const CURLINFO_EFFECTIVE_URL: c_int = 0x100001;
const CURLINFO_RESPONSE_CODE: c_int = 0x200002;
const CURLINFO_PRIVATE: c_int = 0x100015;
const CURLINFO_CONTENT_TYPE: c_int = 0x100012;
const CURLINFO_SIZE_DOWNLOAD_T: c_int = 0x600008;
const CURLINFO_CONTENT_LENGTH_DOWNLOAD_T: c_int = 0x60000f;
const CURLMOPT_MAX_HOST_CONNECTIONS: c_int = 7;
const CURLE_OPERATION_TIMEDOUT: c_int = 28;
const CURLE_ABORTED_BY_CALLBACK: c_int = 42;
const CURL_GLOBAL_DEFAULT: c_long = 3;
const CURLVERSION_NOW: c_int = 11;

/// `CURLMsg`: msg, easy_handle, data (the result code for CURLMSG_DONE).
#[repr(C)]
struct CurlMsg {
    msg: c_int,
    easy_handle: *mut Curl,
    result: c_int,
}

#[allow(non_snake_case)]
struct Lib {
    easy_init: unsafe extern "C" fn() -> *mut Curl,
    easy_setopt: unsafe extern "C" fn(*mut Curl, c_int, ...) -> c_int,
    easy_getinfo: unsafe extern "C" fn(*mut Curl, c_int, ...) -> c_int,
    easy_cleanup: unsafe extern "C" fn(*mut Curl),
    easy_strerror: unsafe extern "C" fn(c_int) -> *const c_char,
    multi_init: unsafe extern "C" fn() -> *mut CurlM,
    multi_setopt: unsafe extern "C" fn(*mut CurlM, c_int, ...) -> c_int,
    multi_add_handle: unsafe extern "C" fn(*mut CurlM, *mut Curl) -> c_int,
    multi_remove_handle: unsafe extern "C" fn(*mut CurlM, *mut Curl) -> c_int,
    multi_perform: unsafe extern "C" fn(*mut CurlM, *mut c_int) -> c_int,
    multi_wait: unsafe extern "C" fn(*mut CurlM, *mut c_void, u32, c_int, *mut c_int) -> c_int,
    multi_info_read: unsafe extern "C" fn(*mut CurlM, *mut c_int) -> *mut CurlMsg,
    multi_cleanup: unsafe extern "C" fn(*mut CurlM) -> c_int,
    slist_append: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    slist_free_all: unsafe extern "C" fn(*mut c_void),
    version_info: unsafe extern "C" fn(c_int) -> *const c_void,
}

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn fopen(path: *const c_char, mode: *const c_char) -> *mut c_void;
    fn fclose(f: *mut c_void) -> c_int;
    fn unlink(path: *const c_char) -> c_int;
}

fn lib() -> Option<&'static Lib> {
    static LIB: std::sync::OnceLock<Option<Lib>> = std::sync::OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let mut h = std::ptr::null_mut();
        for name in [c"libcurl.so.4", c"libcurl.so", c"libcurl-gnutls.so.4"] {
            h = dlopen(name.as_ptr(), 2 /* RTLD_NOW */);
            if !h.is_null() {
                break;
            }
        }
        if h.is_null() {
            return None;
        }
        macro_rules! sym {
            ($n:literal) => {{
                let p = dlsym(h, concat!($n, "\0").as_ptr() as *const c_char);
                if p.is_null() {
                    return None;
                }
                std::mem::transmute::<*mut c_void, _>(p)
            }};
        }
        let global_init: unsafe extern "C" fn(c_long) -> c_int = sym!("curl_global_init");
        global_init(CURL_GLOBAL_DEFAULT);
        Some(Lib {
            easy_init: sym!("curl_easy_init"),
            easy_setopt: sym!("curl_easy_setopt"),
            easy_getinfo: sym!("curl_easy_getinfo"),
            easy_cleanup: sym!("curl_easy_cleanup"),
            easy_strerror: sym!("curl_easy_strerror"),
            multi_init: sym!("curl_multi_init"),
            multi_setopt: sym!("curl_multi_setopt"),
            multi_add_handle: sym!("curl_multi_add_handle"),
            multi_remove_handle: sym!("curl_multi_remove_handle"),
            multi_perform: sym!("curl_multi_perform"),
            multi_wait: sym!("curl_multi_wait"),
            multi_info_read: sym!("curl_multi_info_read"),
            multi_cleanup: sym!("curl_multi_cleanup"),
            slist_append: sym!("curl_slist_append"),
            slist_free_all: sym!("curl_slist_free_all"),
            version_info: sym!("curl_version_info"),
        })
    })
    .as_ref()
}

fn http_errstr(status: c_long) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Request Entity Too Large",
        414 => "Request-URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Requested Range Not Satisfiable",
        417 => "Expectation Failed",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown Error",
    }
}

fn ftp_errstr(status: c_long) -> &'static str {
    match status {
        421 => "Service not available, closing control connection",
        425 => "Cannot open data connection",
        426 => "Connection closed; transfer aborted",
        430 => "Invalid username or password",
        434 => "Requested host unavailable",
        450 => "Requested file action not taken",
        451 => "Requested action aborted; local error in processing",
        452 => "Requested action not taken; insufficient storage space in system",
        501 => "Syntax error in parameters or arguments",
        502 => "Command not implemented",
        503 => "Bad sequence of commands",
        504 => "Command not implemented for that parameter",
        530 => "Not logged in",
        532 => "Need account for storing files",
        550 => "Requested action not taken; file unavailable",
        551 => "Requested action aborted; page type unknown",
        552 => "Requested file action aborted; exceeded storage allocation",
        553 => "Requested action not taken; file name not allowed",
        _ => "Unknown Error",
    }
}

thread_local! {
    /// What libcurl's callbacks print (REprintf), written to the console
    /// after each `curl_multi_perform`.
    static OUT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    /// progress(): the reported total and the dashes printed so far
    static TOTAL: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    static NDASHES: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static CURRENT_TIME: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    static CURRENT_TIMEOUT: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

fn eprint(s: &str) {
    OUT.with(|o| o.borrow_mut().push_str(s));
}

fn flush_out(i: &mut Interp) {
    let s = OUT.with(|o| std::mem::take(&mut *o.borrow_mut()));
    if !s.is_empty() {
        i.write_err(&s);
    }
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

// R: libcurl.c:progress (a progress bar for a single download)
unsafe extern "C" fn progress(clientp: *mut c_void, dltotal: i64, dlnow: i64, _: i64, _: i64) -> c_int {
    let l = lib().unwrap();
    let mut status: c_long = 0;
    (l.easy_getinfo)(clientp, CURLINFO_RESPONSE_CODE, &mut status as *mut c_long);
    if status < 300 && dltotal > 0 {
        if TOTAL.get() == 0.0 {
            let total = dltotal as f64;
            TOTAL.set(total);
            let mut ty: *const c_char = std::ptr::null();
            (l.easy_getinfo)(clientp, CURLINFO_CONTENT_TYPE, &mut ty as *mut *const c_char);
            let ty =
                if ty.is_null() { "unknown".to_string() } else { CStr::from_ptr(ty).to_string_lossy().into_owned() };
            eprint(&format!("Content type '{}'", ty));
            if total > 1024.0 * 1024.0 {
                eprint(&format!(" length {:.0} bytes ({:.1} MB)\n", total, total / 1024.0 / 1024.0));
            } else if total > 10240.0 {
                eprint(&format!(" length {} bytes ({} KB)\n", total as i32, (total / 1024.0) as i32));
            } else {
                eprint(&format!(" length {} bytes\n", total as i32));
            }
        }
        // R: libcurl.c:putdashes
        let new = (50.0 * dlnow as f64 / TOTAL.get()) as i32;
        let old = NDASHES.get();
        if new > old {
            eprint(&"=".repeat((new - old) as usize));
            NDASHES.set(new);
        }
    }
    0
}

// R: libcurl.c:progress_multi (an absolute-time timeout per transfer)
unsafe extern "C" fn progress_multi(clientp: *mut c_void, dltotal: i64, dlnow: i64, _: i64, _: i64) -> c_int {
    let tstart = clientp as *mut f64;
    if !tstart.is_null() {
        if *tstart == 0.0 && (dlnow > 0 || dltotal > 0) {
            *tstart = CURRENT_TIME.get();
        } else if *tstart > 0.0 && (CURRENT_TIME.get() - *tstart) > CURRENT_TIMEOUT.get() as f64 {
            return 1;
        }
    }
    0
}

// R: libcurl.c:prereq_multi
unsafe extern "C" fn prereq_multi(clientp: *mut c_void, _: *mut c_char, _: *mut c_char, _: c_int, _: c_int) -> c_int {
    *(clientp as *mut f64) = CURRENT_TIME.get();
    0
}

/// The download's resources, released however the call ends (R: the
/// context's `download_cleanup`).
struct Download {
    l: &'static Lib,
    headers: *mut c_void,
    mhnd: *mut CurlM,
    hnd: Vec<*mut Curl>,
    out: Vec<*mut c_void>,
    errs: Box<[c_int]>,
    tstart: Box<[f64]>,
    /// the destination paths
    paths: Vec<CString>,
    /// R: `c.sfile` is set only for a mode that writes: failed downloads
    /// are then deleted
    delete_failed: bool,
}

impl Download {
    // R: libcurl.c:download_cleanup_url
    unsafe fn cleanup_url(&mut self, i: usize) {
        let l = self.l;
        if !self.out[i].is_null() {
            fclose(self.out[i]);
            self.out[i] = std::ptr::null_mut();
            let mut dl: i64 = 0;
            (l.easy_getinfo)(self.hnd[i], CURLINFO_SIZE_DOWNLOAD_T, &mut dl as *mut i64);
            if self.delete_failed {
                let mut status: c_long = 0;
                (l.easy_getinfo)(self.hnd[i], CURLINFO_RESPONSE_CODE, &mut status as *mut c_long);
                if status != 200 && dl == 0 {
                    unlink(self.paths[i].as_ptr());
                }
            }
            (l.multi_remove_handle)(self.mhnd, self.hnd[i]);
        }
        if !self.hnd[i].is_null() {
            (l.easy_cleanup)(self.hnd[i]);
            self.hnd[i] = std::ptr::null_mut();
        }
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        unsafe {
            for i in 0..self.hnd.len() {
                self.cleanup_url(i);
            }
            if !self.mhnd.is_null() {
                (self.l.multi_cleanup)(self.mhnd);
            }
            if !self.headers.is_null() {
                (self.l.slist_free_all)(self.headers);
            }
        }
    }
}

struct Opts {
    urls: Vec<CString>,
    mode: CString,
    quiet: bool,
    single: bool,
    user_agent: Option<CString>,
    timeout: c_int,
    verbose: bool,
}

// R: libcurl.c:curlCommon
unsafe fn curl_common(l: &Lib, hnd: *mut Curl, o: &Opts) {
    let capath = std::env::var("CURL_CA_BUNDLE").ok().filter(|s| !s.is_empty());
    if let Some(c) = capath.and_then(|c| CString::new(c).ok()) {
        (l.easy_setopt)(hnd, CURLOPT_CAINFO, c.as_ptr());
    }
    match &o.user_agent {
        Some(ua) => {
            (l.easy_setopt)(hnd, CURLOPT_USERAGENT, ua.as_ptr());
        }
        None => {
            let d = (l.version_info)(CURLVERSION_NOW);
            let v = *((d as *const u8).add(8) as *const *const c_char);
            let mut buf = format!("libcurl/{}", CStr::from_ptr(v).to_string_lossy());
            buf.truncate(19);
            let c = CString::new(buf).unwrap_or_default();
            (l.easy_setopt)(hnd, CURLOPT_USERAGENT, c.as_ptr());
        }
    }
    let timeout: c_long = if o.timeout == NA_INTEGER { 0 } else { 1000 * o.timeout as c_long };
    CURRENT_TIMEOUT.set(if o.timeout == NA_INTEGER { 0 } else { o.timeout });
    (l.easy_setopt)(hnd, CURLOPT_CONNECTTIMEOUT_MS, timeout);
    (l.easy_setopt)(hnd, CURLOPT_TIMEOUT_MS, timeout);
    (l.easy_setopt)(hnd, CURLOPT_FOLLOWLOCATION, 1 as c_long);
    (l.easy_setopt)(hnd, CURLOPT_MAXREDIRS, 20 as c_long);
    if o.verbose {
        (l.easy_setopt)(hnd, CURLOPT_VERBOSE, 1 as c_long);
    }
    (l.easy_setopt)(hnd, CURLOPT_COOKIEFILE, c"".as_ptr());
}

// R: libcurl.c:download_add_url
unsafe fn add_url(i: &mut Interp, k: usize, o: &Opts, mustwork: bool, d: &mut Download) -> RResult<bool> {
    let l = d.l;
    let url = &o.urls[k];
    d.hnd[k] = (l.easy_init)();
    if d.hnd[k].is_null() {
        if mustwork {
            d.errs[k] += 1;
            i.warning("could not create curl handle")?;
        }
        return Ok(false);
    }
    let h = d.hnd[k];
    (l.easy_setopt)(h, CURLOPT_URL, url.as_ptr());
    (l.easy_setopt)(h, CURLOPT_FAILONERROR, 1 as c_long);
    (l.easy_setopt)(h, CURLOPT_PIPEWAIT, 1 as c_long);
    curl_common(l, h, o);
    (l.easy_setopt)(h, CURLOPT_TCP_KEEPALIVE, 1 as c_long);
    (l.easy_setopt)(h, CURLOPT_HTTPHEADER, d.headers);
    let file = &d.paths[k];
    d.out[k] = fopen(file.as_ptr(), o.mode.as_ptr());
    if d.out[k].is_null() {
        if mustwork {
            let e = std::io::Error::last_os_error();
            d.errs[k] += 1;
            i.warning(format!(
                "URL {}: cannot open destfile '{}', reason '{}'",
                url.to_string_lossy(),
                file.to_string_lossy(),
                super::io::os_error_message(&e)
            ))?;
        }
        return Ok(false);
    }
    (l.easy_setopt)(h, CURLOPT_WRITEDATA, d.out[k]);
    (l.multi_add_handle)(d.mhnd, h);
    (l.easy_setopt)(h, CURLOPT_PRIVATE, d.errs.as_mut_ptr().add(k) as *mut c_void);
    TOTAL.set(0.0);
    if !o.quiet && o.single {
        (l.easy_setopt)(h, CURLOPT_NOPROGRESS, 0 as c_long);
        NDASHES.set(0);
        (l.easy_setopt)(
            h,
            CURLOPT_XFERINFOFUNCTION,
            progress as unsafe extern "C" fn(*mut c_void, i64, i64, i64, i64) -> c_int,
        );
        (l.easy_setopt)(h, CURLOPT_XFERINFODATA, h);
    } else if o.quiet && o.single {
        (l.easy_setopt)(h, CURLOPT_NOPROGRESS, 1 as c_long);
    } else {
        (l.easy_setopt)(h, CURLOPT_NOPROGRESS, 0 as c_long);
        (l.easy_setopt)(h, CURLOPT_TIMEOUT, 0 as c_long);
        d.tstart[k] = 0.0;
        let ts = d.tstart.as_mut_ptr().add(k) as *mut c_void;
        (l.easy_setopt)(
            h,
            CURLOPT_XFERINFOFUNCTION,
            progress_multi as unsafe extern "C" fn(*mut c_void, i64, i64, i64, i64) -> c_int,
        );
        (l.easy_setopt)(h, CURLOPT_XFERINFODATA, ts);
        (l.easy_setopt)(
            h,
            CURLOPT_PREREQFUNCTION,
            prereq_multi as unsafe extern "C" fn(*mut c_void, *mut c_char, *mut c_char, c_int, c_int) -> c_int,
        );
        (l.easy_setopt)(h, CURLOPT_PREREQDATA, ts);
    }
    if !o.quiet {
        eprint(&format!("trying URL '{}'\n", url.to_string_lossy()));
        flush_out(i);
    }
    Ok(true)
}

// R: libcurl.c:download_report_url_error
unsafe fn report_url_error(i: &mut Interp, l: &Lib, msg: &CurlMsg) -> RResult<()> {
    let mut url: *const c_char = std::ptr::null();
    let mut status: c_long = 0;
    (l.easy_getinfo)(msg.easy_handle, CURLINFO_EFFECTIVE_URL, &mut url as *mut *const c_char);
    (l.easy_getinfo)(msg.easy_handle, CURLINFO_RESPONSE_CODE, &mut status as *mut c_long);
    let mut url_errs: *mut c_int = std::ptr::null_mut();
    if (l.easy_getinfo)(msg.easy_handle, CURLINFO_PRIVATE, &mut url_errs as *mut *mut c_int) == 0 && !url_errs.is_null()
    {
        *url_errs += 1;
    }
    let url = if url.is_null() { String::new() } else { CStr::from_ptr(url).to_string_lossy().into_owned() };
    if status >= 400 {
        let (strerr, ty) =
            if url.starts_with('h') { (http_errstr(status), "HTTP") } else { (ftp_errstr(status), "FTP") };
        i.warning(format!("cannot open URL '{}': {} status was '{} {}'", url, ty, status, strerr))
    } else {
        let strerr = CStr::from_ptr((l.easy_strerror)(msg.result)).to_string_lossy().into_owned();
        let timedout = msg.result == CURLE_OPERATION_TIMEDOUT
            || msg.result == CURLE_ABORTED_BY_CALLBACK
            || strerr == "Timeout was reached";
        if timedout {
            i.warning(format!("URL '{}': Timeout of {} seconds was reached", url, CURRENT_TIMEOUT.get()))
        } else {
            i.warning(format!("URL '{}': status was '{}'", url, strerr))
        }
    }
}

// R: libcurl.c:download_close_finished
unsafe fn close_finished(i: &mut Interp, d: &mut Download) -> RResult<()> {
    let l = d.l;
    loop {
        let mut n: c_int = 0;
        let msg = (l.multi_info_read)(d.mhnd, &mut n);
        if msg.is_null() {
            break;
        }
        let mut url_errs: *mut c_int = std::ptr::null_mut();
        (l.easy_getinfo)((*msg).easy_handle, CURLINFO_PRIVATE, &mut url_errs as *mut *mut c_int);
        let k = url_errs.offset_from(d.errs.as_ptr()) as usize;
        if (*msg).result != 0 {
            report_url_error(i, l, &*msg)?;
        }
        d.cleanup_url(k);
        if n <= 0 {
            break;
        }
    }
    Ok(())
}

// R: libcurl.c:in_do_curlDownload
fn do_curldownload(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let scmd = match arg(&args, 0) {
        Value::Str(s) if !s.data.is_empty() => s,
        _ => return Err(Flow::error("invalid 'url' argument")),
    };
    let nurls = scmd.data.len();
    let single = nurls == 1;
    const MAX_CONCURRENT_URLS: usize = 15;
    let sfile = match arg(&args, 1) {
        Value::Str(s) if !s.data.is_empty() => s,
        _ => return Err(Flow::error("invalid 'destfile' argument")),
    };
    if sfile.data.len() != nurls {
        return Err(Flow::error("lengths of 'url' and 'destfile' must match"));
    }
    let quiet = as_logical(&arg(&args, 2));
    if quiet == NA_LOGICAL {
        return Err(Flow::error("invalid 'quiet' argument"));
    }
    let mode = match arg(&args, 3) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'mode' argument")),
    };
    let cache_ok = as_logical(&arg(&args, 4));
    if cache_ok == NA_LOGICAL {
        return Err(Flow::error("invalid 'cacheOK' argument"));
    }
    let sheaders = arg(&args, 5);
    if !matches!(sheaders, Value::Null | Value::Str(_)) {
        return Err(Flow::error("invalid 'headers' argument"));
    }
    let Some(l) = lib() else {
        return Err(Flow::error(
            "robigo: download.file(method = \"libcurl\") needs the system's libcurl (libcurl.so.4)",
        ));
    };
    let cs = |s: &RStr| CString::new(s.as_deref().unwrap_or("NA")).unwrap_or_default();
    // GetOption1(HTTPUserAgent): the default "R (...)" agent is libcurl's
    let user_agent = match i.get_option("HTTPUserAgent") {
        Some(Value::Str(s)) if s.data.len() == 1 => {
            let p = s.data[0].as_deref().unwrap_or("NA");
            if p.starts_with("R (") { None } else { CString::new(p).ok() }
        }
        _ => None,
    };
    let verbosity = as_integer(&i.get_option("internet.info").unwrap_or(Value::Null));
    let o = Opts {
        urls: scmd.data.iter().map(cs).collect(),
        mode: CString::new(mode.clone()).unwrap_or_default(),
        quiet: quiet != 0,
        single,
        user_agent,
        timeout: as_integer(&i.get_option("timeout").unwrap_or(Value::Null)),
        verbose: verbosity < 2,
    };
    let paths: Vec<CString> = sfile
        .data
        .iter()
        .map(|f| CString::new(super::io::expand_file_name(f.as_deref().unwrap_or("NA"))).unwrap_or_default())
        .collect();
    let mut d = Download {
        l,
        headers: std::ptr::null_mut(),
        mhnd: std::ptr::null_mut(),
        hnd: vec![std::ptr::null_mut(); nurls],
        out: vec![std::ptr::null_mut(); nurls],
        errs: vec![0; nurls].into_boxed_slice(),
        tstart: vec![0.0; nurls].into_boxed_slice(),
        paths,
        delete_failed: mode.contains('w'),
    };
    unsafe {
        if let Value::Str(h) = &sheaders {
            for x in h.data.iter() {
                let c = cs(x);
                let tmp = (l.slist_append)(d.headers, c.as_ptr());
                if tmp.is_null() {
                    return Err(Flow::error("out of memory"));
                }
                d.headers = tmp;
            }
        }
        if cache_ok == 0 {
            let tmp = (l.slist_append)(d.headers, c"Pragma: no-cache".as_ptr());
            if tmp.is_null() {
                return Err(Flow::error("out of memory"));
            }
            d.headers = tmp;
        }
        d.mhnd = (l.multi_init)();
        if d.mhnd.is_null() {
            return Err(Flow::error("could not create curl handle"));
        }
        (l.multi_setopt)(d.mhnd, CURLMOPT_MAX_HOST_CONNECTIONS, 6 as c_long);
        let mut next_url = 0usize;
        // R: download_add_one_url / download_try_add_urls
        let add_one = |i: &mut Interp, next: &mut usize, d: &mut Download| -> RResult<bool> {
            while *next < nurls {
                let k = *next;
                *next += 1;
                if add_url(i, k, &o, true, d)? {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        let try_add = |i: &mut Interp, next: &mut usize, n: usize, d: &mut Download| -> RResult<()> {
            let mut added = 0;
            while added < n && *next < nurls {
                if add_url(i, *next, &o, false, d)? {
                    *next += 1;
                    added += 1;
                } else {
                    break;
                }
            }
            Ok(())
        };
        if !single {
            CURRENT_TIME.set(now());
        }
        if !add_one(i, &mut next_url, &mut d)? {
            // no destination file could be opened
            return Ok(Value::int(1));
        }
        try_add(i, &mut next_url, MAX_CONCURRENT_URLS - 1, &mut d)?;
        let mut still_running: c_int = 0;
        let mut repeats = 0;
        (l.multi_perform)(d.mhnd, &mut still_running);
        flush_out(i);
        loop {
            if !single {
                CURRENT_TIME.set(now());
            }
            let mut numfds: c_int = 0;
            let mc = (l.multi_wait)(d.mhnd, std::ptr::null_mut(), 0, 100, &mut numfds);
            if mc != 0 {
                i.warning(format!("curl_multi_wait() failed, code {}", mc))?;
                break;
            }
            if numfds == 0 {
                if repeats > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                repeats += 1;
            } else {
                repeats = 0;
            }
            if !single {
                CURRENT_TIME.set(now());
            }
            (l.multi_perform)(d.mhnd, &mut still_running);
            flush_out(i);
            if !single {
                close_finished(i, &mut d)?;
            }
            if still_running == 0 && add_one(i, &mut next_url, &mut d)? {
                still_running += 1;
            }
            try_add(i, &mut next_url, MAX_CONCURRENT_URLS.saturating_sub(still_running as usize), &mut d)?;
            if !single {
                CURRENT_TIME.set(now());
            }
            (l.multi_perform)(d.mhnd, &mut still_running);
            flush_out(i);
            if still_running == 0 && next_url >= nurls {
                break;
            }
        }
        if TOTAL.get() > 0.0 {
            eprint("\n");
        }
        flush_out(i);
        let mut status: c_long = 0;
        if single {
            (l.easy_getinfo)(d.hnd[0], CURLINFO_RESPONSE_CODE, &mut status as *mut c_long);
            let mut dl: i64 = 0;
            (l.easy_getinfo)(d.hnd[0], CURLINFO_SIZE_DOWNLOAD_T, &mut dl as *mut i64);
            if quiet == 0 && status == 200 {
                if dl > 1024 * 1024 {
                    i.write_err(&format!("downloaded {:.1} MB\n\n", dl as f64 / 1024.0 / 1024.0));
                } else if dl > 10240 {
                    i.write_err(&format!("downloaded {} KB\n\n", (dl as f64 / 1024.0) as i32));
                } else {
                    i.write_err(&format!("downloaded {} bytes\n\n", dl as i32));
                }
            }
            let mut cl: i64 = -1;
            (l.easy_getinfo)(d.hnd[0], CURLINFO_CONTENT_LENGTH_DOWNLOAD_T, &mut cl as *mut i64);
            if cl >= 0 && dl != cl {
                i.warning(format!("downloaded length {} != reported length {}", dl, cl))?;
            }
        }
        close_finished(i, &mut d)?;
        let n_err = d.errs.iter().filter(|&&e| e != 0).count();
        let url0 = scmd.data[0].as_deref().unwrap_or("NA").to_string();
        if !single {
            if n_err == nurls {
                return Err(Flow::error("cannot download any files"));
            } else if n_err > 0 {
                i.warning("some files were not downloaded")?;
            }
        } else if n_err > 0 {
            return Err(Flow::error(if status != 200 {
                format!("cannot open URL '{}'", url0)
            } else {
                format!("download from '{}' failed", url0)
            }));
        }
        let mut ans = Value::int(0);
        if nurls > 1 {
            ans.set_attr_raw(
                crate::sym::Sym::new("retvals"),
                Value::ints(d.errs.iter().map(|&e| (e != 0) as i32).collect()),
            );
        }
        Ok(ans)
    }
}
