//! System utilities: environment variables, time, files and directories,
//! locales and version information (GNU R's sysutils.c, platform.c,
//! util.c, times.c, version.c, CommandLineArgs.c, sys-unix.c), plus the
//! `.Machine` and `.Platform` variables (platform.c `Init_R_Variables`).

use std::ffi::{CStr, CString, c_char, c_int, c_long};
use std::sync::Mutex;

use super::{arg, internal};
use crate::coerce::{as_integer, as_logical, as_real};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

use super::io::{expand_file_name, os_error_message};

pub static DEFS: &[Builtin] = &[
    internal("Sys.getenv", do_getenv, 2),
    internal("Sys.setenv", do_setenv, 2).invisible(),
    internal("Sys.unsetenv", do_unsetenv, 1).invisible(),
    internal("Sys.time", do_systime, 0),
    internal("Sys.sleep", do_syssleep, 1).invisible(),
    internal("Sys.getpid", do_sysgetpid, 0),
    internal("Sys.info", do_sysinfo, 0),
    internal("Sys.getlocale", do_getlocale, 1),
    internal("Sys.setlocale", do_setlocale, 2),
    internal("l10n_info", do_l10n_info, 0),
    super::prim("proc.time", do_proctime, 0),
    internal("date", do_date, 0),
    internal("commandArgs", do_command_args, 0),
    internal("Version", do_version, 0),
    internal("R.home", do_rhome, 0),
    internal("gc", do_gc, 3),
    internal("tempdir", do_tempdir, 1),
    internal("tempfile", do_tempfile, 3),
    internal("file.exists", do_fileexists, 1),
    internal("file.remove", do_fileremove, 1),
    internal("file.rename", do_filerename, 2),
    internal("file.create", do_filecreate, 2),
    internal("file.append", do_fileappend, 2),
    internal("unlink", do_unlink, 4).invisible(),
    internal("dir.exists", do_direxists, 1),
    internal("dir.create", do_dircreate, 4).invisible(),
    internal("getwd", do_getwd, 0),
    internal("setwd", do_setwd, 1).invisible(),
    internal("basename", do_basename, 1),
    internal("dirname", do_dirname, 1),
    internal("normalizePath", do_normalizepath, 3),
    internal("path.expand", do_pathexpand, 1),
    internal("gettext", do_gettext, -1),
    internal("ngettext", do_ngettext, 4),
    internal("bindtextdomain", do_bindtextdomain, 2),
];

// ----------------------------------------------------------------------
// Message translation (R: errors.c). rr is built without NLS, so these
// are R's code paths for a build without ENABLE_NLS.

fn do_gettext(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Flow::error_call(call.clone(), "either 2 or 3 arguments are required"));
    }
    Ok(arg(&args, 1))
}

fn do_ngettext(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let msg1 = arg(&args, 1);
    let msg2 = arg(&args, 2);
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER || n < 0 {
        return Err(Flow::error("invalid 'n' argument"));
    }
    if !matches!(&msg1, Value::Str(v) if v.data.len() == 1) {
        return Err(Flow::error("'msg1' must be a character string"));
    }
    if !matches!(&msg2, Value::Str(v) if v.data.len() == 1) {
        return Err(Flow::error("'msg2' must be a character string"));
    }
    Ok(if n == 1 { msg1 } else { msg2 })
}

fn do_bindtextdomain(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Null)
}

unsafe extern "C" {
    fn mkdtemp(template: *mut c_char) -> *mut c_char;
    fn atexit(cb: extern "C" fn()) -> c_int;
    fn time(t: *mut i64) -> i64;
    fn ctime_r(t: *const i64, buf: *mut c_char) -> *mut c_char;
    fn uname(buf: *mut UtsName) -> c_int;
    fn getlogin() -> *mut c_char;
    fn getuid() -> u32;
    fn geteuid() -> u32;
    fn getpwuid(uid: u32) -> *mut Passwd;
    fn getrusage(who: c_int, usage: *mut RUsage) -> c_int;
    fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
    fn nl_langinfo(item: c_int) -> *mut c_char;
}

#[repr(C)]
struct UtsName {
    sysname: [c_char; 65],
    nodename: [c_char; 65],
    release: [c_char; 65],
    version: [c_char; 65],
    machine: [c_char; 65],
    domainname: [c_char; 65],
}

#[repr(C)]
struct Passwd {
    pw_name: *mut c_char,
}

#[repr(C)]
#[derive(Default)]
struct TimeVal {
    tv_sec: c_long,
    tv_usec: c_long,
}

#[repr(C)]
#[derive(Default)]
struct RUsage {
    ru_utime: TimeVal,
    ru_stime: TimeVal,
    rest: [c_long; 14],
}

fn cstr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }
}

fn str_vec(v: &Value, msg: &str) -> RResult<Vec<RStr>> {
    match v {
        Value::Str(s) => Ok(s.data.clone()),
        _ => Err(Flow::error(msg.to_string())),
    }
}

// ----------------------------------------------------------------------
// .Machine, .Platform, R.version

/// R: platform.c:Init_R_Variables (`.Machine` from machine.c).
pub fn install_vars(interp: &mut Interp) {
    let r = Value::real;
    let i = Value::int;
    let machine = Value::named_list(vec![
        ("double.eps", r(f64::EPSILON)),
        ("double.neg.eps", r(f64::EPSILON / 2.0)),
        ("double.xmin", r(f64::MIN_POSITIVE)),
        ("double.xmax", r(f64::MAX)),
        ("double.base", i(2)),
        ("double.digits", i(53)),
        ("double.rounding", i(5)),
        ("double.guard", i(0)),
        ("double.ulp.digits", i(-52)),
        ("double.neg.ulp.digits", i(-53)),
        ("double.exponent", i(11)),
        ("double.min.exp", i(-1022)),
        ("double.max.exp", i(1024)),
        ("integer.max", i(i32::MAX)),
        ("sizeof.long", i(8)),
        ("sizeof.longlong", i(8)),
        ("sizeof.longdouble", i(16)),
        ("sizeof.pointer", i(8)),
        ("sizeof.time_t", i(8)),
        ("longdouble.eps", r(1.0842021724855044e-19)),
        ("longdouble.neg.eps", r(5.421010862427522e-20)),
        ("longdouble.digits", i(64)),
        ("longdouble.rounding", i(5)),
        ("longdouble.guard", i(0)),
        ("longdouble.ulp.digits", i(-63)),
        ("longdouble.neg.ulp.digits", i(-64)),
        ("longdouble.exponent", i(15)),
        ("longdouble.min.exp", i(-16382)),
        ("longdouble.max.exp", i(16384)),
    ]);
    let st = Value::str;
    let platform = Value::named_list(vec![
        ("OS.type", st("unix")),
        ("file.sep", st("/")),
        ("dynlib.ext", st(".so")),
        ("GUI", st("X11")),
        ("endian", st("little")),
        ("pkgType", st("source")),
        ("path.sep", st(":")),
        ("r_arch", st("")),
    ]);
    interp.base.set_local(Sym::new(".Machine"), machine);
    interp.base.set_local(Sym::new(".Platform"), platform);
    // R: library/profile/Common.R
    let mut rv = version_list();
    rv.set_attr_raw(s::CLASS, Value::str("simple.list"));
    interp.base.set_local(Sym::new("R.version"), rv.clone());
    interp.base.set_local(Sym::new("version"), rv);
    interp.base.set_local(Sym::new("R.version.string"), Value::str(VERSION_STRING));
}

const VERSION_STRING: &str = "R version 4.5.0 (2025-04-11)";

fn version_list() -> Value {
    let st = Value::str;
    let arch = std::env::consts::ARCH;
    let os = "linux-gnu";
    Value::named_list(vec![
        ("platform", st(&format!("{}-pc-{}", arch, os))),
        ("arch", st(arch)),
        ("os", st(os)),
        ("system", st(&format!("{}, {}", arch, os))),
        ("status", st("")),
        ("major", st("4")),
        ("minor", st("5.0")),
        ("year", st("2025")),
        ("month", st("04")),
        ("day", st("11")),
        ("svn rev", st("88135")),
        ("language", st("R")),
        ("version.string", st(VERSION_STRING)),
        ("nickname", st("How About a Twenty-Six")),
    ])
}

fn do_version(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(version_list())
}

/// `commandArgs()`: the arguments the way `Rscript` passes them to R.
fn do_command_args(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let argv: Vec<String> = std::env::args().collect();
    let mut out: Vec<String> = Vec::new();
    out.push(argv.first().cloned().unwrap_or_else(|| "rr".into()));
    out.push("--no-echo".into());
    out.push("--no-restore".into());
    let mut k = 1;
    while k < argv.len() {
        let a = &argv[k];
        match a.as_str() {
            "-e" => {
                out.push("-e".into());
                if let Some(e) = argv.get(k + 1) {
                    out.push(e.clone());
                }
                k += 2;
                continue;
            }
            a if a.starts_with('-') => out.push(a.to_string()),
            _ => {
                out.push(format!("--file={}", a));
                if k + 1 < argv.len() {
                    out.push("--args".into());
                    out.extend(argv[k + 1..].iter().cloned());
                }
                break;
            }
        }
        k += 1;
    }
    Ok(Value::strs_from(out))
}

fn do_rhome(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    match std::env::var("R_HOME") {
        Ok(h) if !h.is_empty() => Ok(Value::str(&h)),
        _ => Err(Flow::error("rr: R.home() is not available: R_HOME is not set")),
    }
}

fn do_gc(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    // rr frees memory by reference counting; there is no collector whose
    // statistics gc() could report.
    Err(Flow::error("rr: 'gc' is not implemented (rr uses reference counting)"))
}

// ----------------------------------------------------------------------
// Environment variables

fn do_getenv(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = str_vec(&arg(&args, 0), "wrong type for argument")?;
    let unset = match arg(&args, 1) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].clone(),
        _ => return Err(Flow::error("wrong type for argument")),
    };
    if x.is_empty() {
        let all: Vec<String> = std::env::vars_os()
            .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
            .collect();
        return Ok(Value::strs_from(all));
    }
    Ok(Value::strs(
        x.iter()
            .map(|n| match std::env::var_os(n.as_deref().unwrap_or("NA")) {
                Some(v) => rstr(&v.to_string_lossy()),
                None => unset.clone(),
            })
            .collect(),
    ))
}

fn do_setenv(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let nm = str_vec(&arg(&args, 0), "wrong type for argument")?;
    let val = str_vec(&arg(&args, 1), "wrong type for argument")?;
    if nm.len() != val.len() {
        return Err(Flow::error("'names' and 'val' are of different lengths"));
    }
    let mut out = Vec::with_capacity(nm.len());
    for (n, v) in nm.iter().zip(val.iter()) {
        let n = n.as_deref().unwrap_or("NA");
        let v = v.as_deref().unwrap_or("NA");
        let ok = !n.is_empty() && !n.contains('=') && !n.contains('\0') && !v.contains('\0');
        if ok {
            // SAFETY: rr is single-threaded.
            unsafe { std::env::set_var(n, v) };
        }
        out.push(ok as Lgl);
    }
    Ok(Value::lgls(out))
}

fn do_unsetenv(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let nm = str_vec(&arg(&args, 0), "wrong type for argument")?;
    let mut out = Vec::with_capacity(nm.len());
    for n in &nm {
        let n = n.as_deref().unwrap_or("NA");
        if !n.is_empty() && !n.contains('=') && !n.contains('\0') {
            // SAFETY: rr is single-threaded.
            unsafe { std::env::remove_var(n) };
        }
        out.push(std::env::var_os(n).is_none() as Lgl);
    }
    Ok(Value::lgls(out))
}

// ----------------------------------------------------------------------
// Time

/// R: times.c:currentTime
pub fn current_time() -> f64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as f64 + 1e-9 * d.subsec_nanos() as f64,
        Err(_) => NA_REAL,
    }
}

fn do_systime(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut v = Value::real(current_time());
    v.set_attr_raw(s::CLASS, Value::strs_from(["POSIXct", "POSIXt"]));
    Ok(v)
}

fn do_syssleep(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let t = as_real(&arg(&args, 0));
    if t.is_nan() || t < 0.0 {
        return Err(Flow::error("invalid 'time' value"));
    }
    std::thread::sleep(std::time::Duration::from_secs_f64(t));
    Ok(Value::Null)
}

fn do_sysgetpid(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::int(std::process::id() as i32))
}

/// R: sysutils.c:do_proctime / R_getProcTime
fn do_proctime(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let et = i.start_time.elapsed().as_secs_f64();
    let mut selfu = RUsage::default();
    let mut child = RUsage::default();
    unsafe {
        getrusage(0, &mut selfu);
        getrusage(-1, &mut child);
    }
    let t = |tv: &TimeVal| tv.tv_sec as f64 + 1e-3 * (tv.tv_usec / 1000) as f64;
    let data = vec![
        t(&selfu.ru_utime),
        t(&selfu.ru_stime),
        1e-3 * (1000.0 * et).round_ties_even(),
        t(&child.ru_utime),
        t(&child.ru_stime),
    ];
    let mut v = Value::reals(data);
    v.set_attr_raw(s::NAMES, Value::strs_from(["user.self", "sys.self", "elapsed", "user.child", "sys.child"]));
    v.set_attr_raw(s::CLASS, Value::str("proc_time"));
    Ok(v)
}

/// R: platform.c:do_date (`ctime()` without the newline).
fn do_date(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut buf = [0 as c_char; 64];
    let s = unsafe {
        let mut t: i64 = 0;
        time(&mut t);
        ctime_r(&t, buf.as_mut_ptr());
        cstr(buf.as_ptr()).unwrap_or_default()
    };
    Ok(Value::str(s.trim_end_matches('\n')))
}

fn do_sysinfo(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut u: UtsName = unsafe { std::mem::zeroed() };
    if unsafe { uname(&mut u) } == -1 {
        return Ok(Value::Null);
    }
    let pw = |uid: u32| -> String {
        let p = unsafe { getpwuid(uid) };
        if p.is_null() {
            "unknown".into()
        } else {
            cstr(unsafe { (*p).pw_name }).unwrap_or_else(|| "unknown".into())
        }
    };
    let vals = vec![
        cstr(u.sysname.as_ptr()).unwrap_or_default(),
        cstr(u.release.as_ptr()).unwrap_or_default(),
        cstr(u.version.as_ptr()).unwrap_or_default(),
        cstr(u.nodename.as_ptr()).unwrap_or_default(),
        cstr(u.machine.as_ptr()).unwrap_or_default(),
        cstr(unsafe { getlogin() }).unwrap_or_else(|| "unknown".into()),
        pw(unsafe { getuid() }),
        pw(unsafe { geteuid() }),
    ];
    let mut v = Value::strs_from(vals);
    v.set_attr_raw(
        s::NAMES,
        Value::strs_from(["sysname", "release", "version", "nodename", "machine", "login", "user", "effective_user"]),
    );
    Ok(v)
}

// ----------------------------------------------------------------------
// Locales

const LC_CTYPE: c_int = 0;
const LC_NUMERIC: c_int = 1;
const LC_TIME: c_int = 2;
const LC_COLLATE: c_int = 3;
const LC_MONETARY: c_int = 4;
const LC_MESSAGES: c_int = 5;
const LC_ALL: c_int = 6;
const LC_PAPER: c_int = 7;
const LC_MEASUREMENT: c_int = 11;
const CODESET: c_int = 14;

/// R sets these categories from the environment at startup.
fn init_locale() {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let empty = CString::new("").unwrap();
    for cat in [LC_CTYPE, LC_COLLATE, LC_TIME, LC_MONETARY, LC_MESSAGES, LC_PAPER, LC_MEASUREMENT] {
        unsafe { setlocale(cat, empty.as_ptr()) };
    }
}

fn locale_category(code: i32) -> Option<c_int> {
    Some(match code {
        1 => LC_ALL,
        2 => LC_COLLATE,
        3 => LC_CTYPE,
        4 => LC_MONETARY,
        5 => LC_NUMERIC,
        6 => LC_TIME,
        7 => LC_MESSAGES,
        8 => LC_PAPER,
        9 => LC_MEASUREMENT,
        _ => return None,
    })
}

fn do_getlocale(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    init_locale();
    let cat = as_integer(&arg(&args, 0));
    if cat == NA_INTEGER || cat < 0 {
        return Err(Flow::error("invalid 'category' argument"));
    }
    let p = locale_category(cat).and_then(|c| cstr(unsafe { setlocale(c, std::ptr::null()) }));
    Ok(Value::str(&p.unwrap_or_default()))
}

fn do_setlocale(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    init_locale();
    let cat = as_integer(&arg(&args, 0));
    if cat == NA_INTEGER || cat < 0 {
        return Err(Flow::error("invalid 'category' argument"));
    }
    let l = match arg(&args, 1) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'locale' argument")),
    };
    let cl = CString::new(l.clone()).unwrap_or_default();
    let p: Option<String> = unsafe {
        match cat {
            1 => {
                if setlocale(LC_CTYPE, cl.as_ptr()).is_null() {
                    None
                } else {
                    setlocale(LC_COLLATE, cl.as_ptr());
                    setlocale(LC_MONETARY, cl.as_ptr());
                    setlocale(LC_TIME, cl.as_ptr());
                    cstr(setlocale(LC_ALL, std::ptr::null()))
                }
            }
            5 => {
                if l != "C" {
                    i.warning("setting 'LC_NUMERIC' may cause R to function strangely")?;
                }
                cstr(setlocale(LC_NUMERIC, cl.as_ptr()))
            }
            2 | 3 | 4 | 6 | 7 | 8 | 9 => cstr(setlocale(locale_category(cat).unwrap(), cl.as_ptr())),
            _ => return Err(Flow::error("invalid 'category' argument")),
        }
    };
    match p {
        Some(p) => Ok(Value::str(&p)),
        None => {
            i.warning(format!("OS reports request to set locale to \"{}\" cannot be honored", l))?;
            Ok(Value::str(""))
        }
    }
}

fn do_l10n_info(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    init_locale();
    let codeset = cstr(unsafe { nl_langinfo(CODESET) }).unwrap_or_default();
    let lc = codeset.to_ascii_lowercase();
    let utf8 = lc == "utf-8" || lc == "utf8";
    let latin1 = lc == "iso-8859-1" || lc == "iso8859-1" || lc == "latin1";
    Ok(Value::named_list(vec![
        ("MBCS", Value::lgl(utf8)),
        ("UTF-8", Value::lgl(utf8)),
        ("Latin-1", Value::lgl(latin1)),
        ("codeset", Value::str(&codeset)),
    ]))
}

// ----------------------------------------------------------------------
// Temporary files

static TEMPDIR: Mutex<Option<String>> = Mutex::new(None);

extern "C" fn clean_tempdir() {
    if let Ok(g) = TEMPDIR.lock() {
        if let Some(d) = g.as_ref() {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

/// The per-session temporary directory (R's `R_TempDir`), created on
/// first use and removed at exit.
pub fn session_tempdir() -> RResult<String> {
    let mut g = TEMPDIR.lock().unwrap();
    if let Some(d) = g.as_ref() {
        return Ok(d.clone());
    }
    let mut base = String::new();
    for v in ["TMPDIR", "TMP", "TEMP"] {
        if let Ok(t) = std::env::var(v) {
            if !t.is_empty() && std::path::Path::new(&t).is_dir() {
                base = t;
                break;
            }
        }
    }
    if base.is_empty() {
        base = "/tmp".into();
    }
    let template = CString::new(format!("{}/RtmpXXXXXX", base)).unwrap();
    let mut bytes = template.into_bytes_with_nul();
    let p = unsafe { mkdtemp(bytes.as_mut_ptr() as *mut c_char) };
    if p.is_null() {
        return Err(Flow::error("cannot create 'R_TempDir'"));
    }
    let d = cstr(p).unwrap();
    *g = Some(d.clone());
    unsafe { atexit(clean_tempdir) };
    Ok(d)
}

fn random_u32() -> u32 {
    thread_local! {
        static STATE: std::cell::Cell<u64> = std::cell::Cell::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E3779B97F4A7C15)
                | 1,
        );
    }
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 33) as u32
    })
}

/// R: sysutils.c:R_tmpnam2
pub fn tempfile_name(tempdir: &str, prefix: &str, fileext: &str) -> String {
    let pid = std::process::id();
    loop {
        let r = random_u32() & 0x7fff_ffff;
        let name = format!("{}/{}{:x}{:x}{}", tempdir, prefix, pid, r, fileext);
        if std::fs::symlink_metadata(&name).is_err() {
            return name;
        }
    }
}

fn do_tempdir(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let check = as_logical(&arg(&args, 0));
    if check == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "NA in coercion to boolean"));
    }
    if check != 0 {
        let mut g = TEMPDIR.lock().unwrap();
        if let Some(d) = g.as_ref() {
            if !std::path::Path::new(d).is_dir() {
                *g = None;
            }
        }
    }
    Ok(Value::str(&session_tempdir()?))
}

fn do_tempfile(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let pattern = str_vec(&arg(&args, 0), "invalid filename pattern")?;
    let tempdir = str_vec(&arg(&args, 1), "invalid 'tempdir' value")?;
    let fileext = str_vec(&arg(&args, 2), "invalid file extension")?;
    if pattern.is_empty() {
        return Err(Flow::error("no 'pattern'"));
    }
    if tempdir.is_empty() {
        return Err(Flow::error("no 'tempdir'"));
    }
    if fileext.is_empty() {
        return Err(Flow::error("no 'fileext'"));
    }
    let n = pattern.len().max(tempdir.len()).max(fileext.len());
    let out: Vec<String> = (0..n)
        .map(|k| {
            tempfile_name(
                tempdir[k % tempdir.len()].as_deref().unwrap_or("NA"),
                pattern[k % pattern.len()].as_deref().unwrap_or("NA"),
                fileext[k % fileext.len()].as_deref().unwrap_or("NA"),
            )
        })
        .collect();
    Ok(Value::strs_from(out))
}

// ----------------------------------------------------------------------
// Files and directories

fn do_fileexists(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let f = str_vec(&arg(&args, 0), "invalid 'file' argument")?;
    Ok(Value::lgls(
        f.iter()
            .map(|p| match p {
                Some(p) => std::fs::metadata(expand_file_name(p)).is_ok() as Lgl,
                None => FALSE,
            })
            .collect(),
    ))
}

fn do_fileremove(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let f = str_vec(&arg(&args, 0), "invalid first filename")?;
    let mut out = Vec::with_capacity(f.len());
    for p in &f {
        match p {
            Some(p) => {
                let path = expand_file_name(p);
                let r = match std::fs::symlink_metadata(&path) {
                    Ok(m) if m.is_dir() => std::fs::remove_dir(&path),
                    _ => std::fs::remove_file(&path),
                };
                match r {
                    Ok(()) => out.push(TRUE),
                    Err(e) => {
                        out.push(FALSE);
                        i.warning(format!("cannot remove file '{}', reason '{}'", p, os_error_message(&e)))?;
                    }
                }
            }
            None => out.push(FALSE),
        }
    }
    Ok(Value::lgls(out))
}

fn do_filerename(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let f1 = str_vec(&arg(&args, 0), "invalid 'from' argument")?;
    let f2 = str_vec(&arg(&args, 1), "invalid 'to' argument")?;
    if f1.len() != f2.len() {
        return Err(Flow::error("'from' and 'to' are of different lengths"));
    }
    let mut out = Vec::with_capacity(f1.len());
    for (a, b) in f1.iter().zip(f2.iter()) {
        let (Some(a), Some(b)) = (a, b) else {
            out.push(FALSE);
            continue;
        };
        let (from, to) = (expand_file_name(a), expand_file_name(b));
        match std::fs::rename(&from, &to) {
            Ok(()) => out.push(TRUE),
            Err(e) => {
                i.warning(format!("cannot rename file '{}' to '{}', reason '{}'", from, to, os_error_message(&e)))?;
                out.push(FALSE);
            }
        }
    }
    Ok(Value::lgls(out))
}

fn do_filecreate(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let f = str_vec(&arg(&args, 0), "invalid filename argument")?;
    let show = as_logical(&arg(&args, 1));
    let show = show != NA_LOGICAL && show != 0;
    let mut out = Vec::with_capacity(f.len());
    for p in &f {
        let Some(p) = p else {
            out.push(FALSE);
            continue;
        };
        match std::fs::File::create(expand_file_name(p)) {
            Ok(_) => out.push(TRUE),
            Err(e) => {
                out.push(FALSE);
                if show {
                    i.warning(format!("cannot create file '{}', reason '{}'", p, os_error_message(&e)))?;
                }
            }
        }
    }
    Ok(Value::lgls(out))
}

/// R: platform.c:do_fileappend
fn do_fileappend(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let f1 = str_vec(&arg(&args, 0), "invalid 'file1' argument")?;
    let f2 = str_vec(&arg(&args, 1), "invalid 'file2' argument")?;
    let (n1, n2) = (f1.len(), f2.len());
    if n1 < 1 {
        return Err(Flow::error("nothing to append to"));
    }
    if n2 < 1 {
        return Ok(Value::lgls(vec![]));
    }
    let n = n1.max(n2);
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let (a, b) = (&f1[k % n1], &f2[k % n2]);
        let ok = match (a, b) {
            (Some(a), Some(b)) => (|| -> std::io::Result<()> {
                let data = std::fs::read(expand_file_name(b))?;
                let mut f = std::fs::OpenOptions::new().append(true).create(true).open(expand_file_name(a))?;
                use std::io::Write;
                f.write_all(&data)
            })()
            .is_ok(),
            _ => false,
        };
        out.push(ok as Lgl);
    }
    Ok(Value::lgls(out))
}

/// R: platform.c:R_unlink
fn r_unlink(name: &str, recursive: bool) -> i32 {
    if name == "." || name == ".." {
        return 0;
    }
    let md = std::fs::symlink_metadata(name);
    if let (Ok(m), true) = (&md, recursive) {
        if m.is_dir() {
            let mut ans = 0;
            match std::fs::read_dir(name) {
                Ok(rd) => {
                    for e in rd.flatten() {
                        let p = format!("{}/{}", name.trim_end_matches('/'), e.file_name().to_string_lossy());
                        let isdir = std::fs::symlink_metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
                        if isdir {
                            ans += r_unlink(&p, recursive);
                        } else {
                            ans += std::fs::remove_file(&p).is_err() as i32;
                        }
                    }
                }
                Err(_) => ans += 1,
            }
            ans += std::fs::remove_dir(name).is_err() as i32;
            return ans;
        }
    }
    let res2 = std::fs::remove_file(name);
    if res2.is_ok() || md.is_err() { 0 } else { 1 }
}

/// Shell-style wildcard match (`*`, `?`, `[...]`) for one path component.
fn wildcard_match(p: &[char], t: &[char]) -> bool {
    match p.first() {
        None => t.is_empty(),
        Some('*') => (0..=t.len()).any(|k| wildcard_match(&p[1..], &t[k..])),
        Some('?') => !t.is_empty() && wildcard_match(&p[1..], &t[1..]),
        Some('[') => {
            let Some(close) = p.iter().skip(1).position(|&c| c == ']').map(|k| k + 1) else {
                return !t.is_empty() && t[0] == '[' && wildcard_match(&p[1..], &t[1..]);
            };
            if t.is_empty() {
                return false;
            }
            let set = &p[1..close];
            let (neg, set) = if set.first() == Some(&'!') || set.first() == Some(&'^') { (true, &set[1..]) } else { (false, set) };
            let mut found = false;
            let mut k = 0;
            while k < set.len() {
                if k + 2 < set.len() && set[k + 1] == '-' {
                    if set[k] <= t[0] && t[0] <= set[k + 2] {
                        found = true;
                    }
                    k += 3;
                } else {
                    if set[k] == t[0] {
                        found = true;
                    }
                    k += 1;
                }
            }
            found != neg && wildcard_match(&p[close + 1..], &t[1..])
        }
        Some(c) => !t.is_empty() && t[0] == *c && wildcard_match(&p[1..], &t[1..]),
    }
}

/// `glob(pattern, GLOB_NOCHECK)`: matching paths, sorted, or the pattern.
fn glob_nocheck(pattern: &str) -> Vec<String> {
    if !pattern.contains(['*', '?', '[']) {
        return vec![pattern.to_string()];
    }
    let absolute = pattern.starts_with('/');
    let comps: Vec<&str> = pattern.split('/').filter(|c| !c.is_empty()).collect();
    let mut cur: Vec<String> = vec![if absolute { "/".into() } else { String::new() }];
    for comp in comps {
        let mut next = Vec::new();
        for base in &cur {
            let join = |n: &str| -> String {
                if base.is_empty() {
                    n.to_string()
                } else if base.ends_with('/') {
                    format!("{}{}", base, n)
                } else {
                    format!("{}/{}", base, n)
                }
            };
            if comp.contains(['*', '?', '[']) {
                let dir = if base.is_empty() { "." } else { base.as_str() };
                let pc: Vec<char> = comp.chars().collect();
                let mut names: Vec<String> = match std::fs::read_dir(dir) {
                    Ok(rd) => rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
                    Err(_) => vec![],
                };
                names.sort();
                for n in names {
                    if n.starts_with('.') && !comp.starts_with('.') {
                        continue;
                    }
                    let nc: Vec<char> = n.chars().collect();
                    if wildcard_match(&pc, &nc) {
                        next.push(join(&n));
                    }
                }
            } else {
                next.push(join(comp));
            }
        }
        cur = next;
    }
    cur.retain(|p| std::fs::symlink_metadata(p).is_ok());
    if cur.is_empty() {
        vec![pattern.to_string()]
    } else {
        cur
    }
}

fn do_unlink(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let fnv = arg(&args, 0);
    let n = fnv.len();
    let mut failures = 0;
    if n > 0 {
        let Value::Str(f) = &fnv else { return Err(Flow::error("invalid 'x' argument")) };
        let recursive = as_logical(&arg(&args, 1));
        if recursive == NA_LOGICAL {
            return Err(Flow::error("invalid 'recursive' argument"));
        }
        if as_logical(&arg(&args, 2)) == NA_LOGICAL {
            return Err(Flow::error("invalid 'force' argument"));
        }
        let expand = as_logical(&arg(&args, 3));
        if expand == NA_LOGICAL {
            return Err(Flow::error("invalid 'expand' argument"));
        }
        for p in &f.data {
            match p {
                Some(p) => {
                    if &**p == "~" {
                        continue;
                    }
                    if expand != 0 {
                        for g in glob_nocheck(&expand_file_name(p)) {
                            failures += r_unlink(&g, recursive != 0);
                        }
                    } else {
                        failures += r_unlink(p, recursive != 0);
                    }
                }
                None => failures += 1,
            }
        }
    }
    Ok(Value::int(if failures > 0 { 1 } else { 0 }))
}

fn do_direxists(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let f = str_vec(&arg(&args, 0), "invalid filename argument")?;
    Ok(Value::lgls(
        f.iter()
            .map(|p| match p {
                Some(p) => std::fs::metadata(expand_file_name(p)).map(|m| m.is_dir()).unwrap_or(false) as Lgl,
                None => FALSE,
            })
            .collect(),
    ))
}

/// R: platform.c:do_dircreate
fn do_dircreate(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    use std::os::unix::fs::DirBuilderExt;
    let path = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].clone(),
        _ => return Err(Flow::error("invalid 'path' argument")),
    };
    let Some(path) = path else { return Ok(Value::lgl(false)) };
    let show = as_logical(&arg(&args, 1));
    let show = show != NA_LOGICAL && show != 0;
    let recursive = as_logical(&arg(&args, 2));
    let recursive = recursive != NA_LOGICAL && recursive != 0;
    let mode = as_integer(&arg(&args, 3));
    let mode = if mode == NA_INTEGER { 0o777 } else { mode as u32 };
    let mut dir = expand_file_name(&path);
    if dir.is_empty() {
        return Err(Flow::error("zero-length 'path' argument"));
    }
    while dir.len() > 1 && dir.ends_with('/') {
        dir.pop();
    }
    let mk = |d: &str| std::fs::DirBuilder::new().mode(mode).create(d);
    if recursive {
        let bytes: Vec<usize> = dir.match_indices('/').map(|(k, _)| k).filter(|&k| k > 0).collect();
        for k in bytes {
            let sub = &dir[..k];
            match std::fs::metadata(sub) {
                Ok(m) => {
                    if !m.is_dir() {
                        if show {
                            i.warning(format!("cannot create dir '{}', reason '{}'", dir, "Not a directory"))?;
                        }
                        return Ok(Value::lgl(false));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if let Err(e) = mk(sub) {
                        if e.kind() != std::io::ErrorKind::AlreadyExists {
                            if show {
                                i.warning(format!("cannot create dir '{}', reason '{}'", dir, os_error_message(&e)))?;
                            }
                            return Ok(Value::lgl(false));
                        }
                    }
                }
                Err(e) => {
                    if show {
                        i.warning(format!("cannot create dir '{}', reason '{}'", dir, os_error_message(&e)))?;
                    }
                    return Ok(Value::lgl(false));
                }
            }
        }
    }
    match mk(&dir) {
        Ok(()) => Ok(Value::lgl(true)),
        Err(e) => {
            if show {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    i.warning(format!("'{}' already exists", dir))?;
                } else {
                    i.warning(format!("cannot create dir '{}', reason '{}'", dir, os_error_message(&e)))?;
                }
            }
            Ok(Value::lgl(false))
        }
    }
}

fn do_getwd(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    match std::env::current_dir() {
        Ok(p) => Ok(Value::str(&p.to_string_lossy())),
        Err(_) => Ok(Value::Null),
    }
}

fn do_setwd(i: &mut Interp, _: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let s = match arg(&args, 0) {
        Value::Str(s) if !s.data.is_empty() => s.data[0].clone(),
        _ => return Err(Flow::error("character argument expected")),
    };
    let Some(s) = s else { return Err(Flow::error("missing value is invalid")) };
    let wd = do_getwd(i, &Value::Null, vec![], env)?;
    if std::env::set_current_dir(expand_file_name(&s)).is_err() {
        return Err(Flow::error("cannot change working directory"));
    }
    Ok(wd)
}

fn do_basename(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let s = str_vec(&arg(&args, 0), "a character vector argument expected")?;
    Ok(Value::strs(
        s.iter()
            .map(|p| {
                p.as_ref().map(|p| {
                    let pp = expand_file_name(p);
                    let t = pp.trim_end_matches(['/', '\\']);
                    let start = t.rfind(['/', '\\']).map(|k| k + 1).unwrap_or(0);
                    std::rc::Rc::from(&t[start..])
                })
            })
            .collect(),
    ))
}

fn do_dirname(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let s = str_vec(&arg(&args, 0), "a character vector argument expected")?;
    Ok(Value::strs(
        s.iter()
            .map(|p| {
                p.as_ref().map(|p| {
                    let pp = expand_file_name(p);
                    if pp.is_empty() {
                        return std::rc::Rc::from("");
                    }
                    let t = pp.trim_end_matches('/');
                    if t.is_empty() {
                        return std::rc::Rc::from("/");
                    }
                    let Some(k) = t.rfind('/') else { return std::rc::Rc::from(".") };
                    let d = t[..k].trim_end_matches('/');
                    if d.is_empty() { std::rc::Rc::from("/") } else { std::rc::Rc::from(d) }
                })
            })
            .collect(),
    ))
}

fn do_normalizepath(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let paths = str_vec(&arg(&args, 0), "'path' must be a character vector")?;
    let must_work = as_logical(&arg(&args, 2));
    let mut out = Vec::with_capacity(paths.len());
    for (k, p) in paths.iter().enumerate() {
        let Some(path) = p else {
            out.push(None);
            if must_work == TRUE {
                return Err(Flow::error(format!("path[{}]=NA", k + 1)));
            } else if must_work == NA_LOGICAL {
                i.warning(format!("path[{}]=NA", k + 1))?;
            }
            continue;
        };
        match std::fs::canonicalize(&**path) {
            Ok(a) => out.push(rstr(&a.to_string_lossy())),
            Err(e) => {
                out.push(p.clone());
                let msg = format!("path[{}]=\"{}\": {}", k + 1, path, os_error_message(&e));
                if must_work == TRUE {
                    return Err(Flow::error(msg));
                } else if must_work == NA_LOGICAL {
                    i.warning(msg)?;
                }
            }
        }
    }
    Ok(Value::strs(out))
}

fn do_pathexpand(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let f = str_vec(&arg(&args, 0), "invalid 'path' argument")?;
    Ok(Value::strs(f.iter().map(|p| p.as_ref().map(|p| std::rc::Rc::from(expand_file_name(p).as_str()))).collect()))
}
