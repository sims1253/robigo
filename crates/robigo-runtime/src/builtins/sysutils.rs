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
use crate::sym::{Sym, s};
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
    super::prim("proc.time", do_proctime, 0),
    internal("date", do_date, 0),
    internal("commandArgs", do_command_args, 0),
    internal("R.home", do_rhome, 0),
    internal("tempdir", do_tempdir, 1),
    internal("tempfile", do_tempfile, 3),
    internal("file.exists", do_fileexists, 1),
    internal("setFileTime", do_setfiletime, 2),
    internal("eSoftVersion", do_esoftversion, 0),
    internal("file.access", do_fileaccess, 2),
    internal("file.info", do_fileinfo, 2),
    internal("file.copy", do_filecopy, 6),
    internal("list.files", do_listfiles, 8),
    internal("list.dirs", do_listdirs, 3),
    internal("file.remove", do_fileremove, 1),
    internal("file.rename", do_filerename, 2),
    internal("file.create", do_filecreate, 2),
    internal("file.append", do_fileappend, 2),
    internal("unlink", do_unlink, 4).invisible(),
    internal("dir.exists", do_direxists, 1),
    internal("dir.create", do_dircreate, 4).invisible(),
    internal("getwd", do_getwd, 0),
    internal("setwd", do_setwd, 1).invisible(),
    internal("normalizePath", do_normalizepath, 3),
    internal("path.expand", do_pathexpand, 1),
    internal("readRenviron", do_read_renviron, 1),
    internal("Sys.glob", do_glob, 2),
    internal("Sys.chmod", do_syschmod, 3).invisible(),
    internal("Sys.readlink", do_readlink, 1),
    internal("Sys.localeconv", do_localeconv, 0),
    internal("system", do_system, 4).custom_vis(),
    internal("capabilities", do_capabilities, 0),
    internal("capabilitiesX11", do_capabilities_x11, 0),
];

/// What this build of robigo supports, in GNU R's order.
// R: platform.c:do_capabilities
fn do_capabilities(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let caps: [(&str, bool); 19] = [
        ("jpeg", false),
        ("png", false),
        ("tiff", false),
        ("tcltk", false),
        ("X11", false),
        ("aqua", false),
        ("http/ftp", false),
        ("sockets", false),
        ("libxml", false),
        ("fifo", true),
        ("cledit", false),
        ("iconv", true),
        ("NLS", false),
        ("Rprof", false),
        ("profmem", false),
        ("cairo", false),
        ("ICU", false),
        ("long.double", true),
        ("libcurl", false),
    ];
    let mut v = Value::lgls(caps.iter().map(|(_, b)| *b as i32).collect());
    v.set_attr_raw(s::NAMES, Value::strs_from(caps.iter().map(|(n, _)| *n)));
    Ok(v)
}

fn do_capabilities_x11(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::lgl(false))
}

/// The exit status of a finished shell command, as R reports it.
fn exit_status(st: &std::process::ExitStatus) -> i32 {
    st.code().unwrap_or(0)
}

// R: unix/sys-unix.c:do_system
fn do_system(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let cmd = match arg(&args, 0) {
        Value::Str(s) if !s.data.is_empty() && s.data[0].is_some() => s.data[0].clone().unwrap().to_string(),
        _ => return Err(Flow::error("non-empty character argument expected")),
    };
    let intern = as_logical(&arg(&args, 1));
    if intern == NA_LOGICAL {
        return Err(Flow::error("'intern' must be logical and not NA"));
    }
    let timeout = as_integer(&arg(&args, 2));
    if timeout == NA_INTEGER || timeout < 0 {
        return Err(Flow::error("invalid 'timeout' argument"));
    }
    if as_logical(&arg(&args, 3)) == NA_LOGICAL {
        return Err(Flow::error("'receive.console.signals' must be logical and not NA"));
    }
    if timeout > 0 {
        return Err(Flow::error("robigo: system() with a timeout is not implemented yet"));
    }
    i.flush();
    let mut command = std::process::Command::new("/bin/sh");
    command.arg("-c").arg(&cmd);
    if intern != 0 {
        let out = match command.stdout(std::process::Stdio::piped()).output() {
            Ok(o) => o,
            Err(e) => {
                return Err(Flow::error(format!(
                    "cannot popen '{}', probable reason '{}'",
                    cmd,
                    super::io::os_error_message(&e)
                )));
            }
        };
        let res = exit_status(&out.status);
        if res & 0xff == 127 {
            return Err(Flow::error("error in running command"));
        }
        if res != 0 {
            i.warning(format!("running command '{}' had status {}", cmd, res))?;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut v = Value::strs_from(text.lines());
        if res != 0 {
            v.set_attr_raw(Sym::new("status"), Value::int(res));
        }
        i.visible = true;
        return Ok(v);
    }
    let res = match command.status() {
        Ok(st) => exit_status(&st),
        Err(_) => 127,
    };
    if res == 127 {
        i.warning("error in running command")?;
    }
    i.visible = false;
    Ok(Value::int(res))
}

/// glibc's `glob_t`.
#[repr(C)]
struct GlobT {
    gl_pathc: usize,
    gl_pathv: *mut *mut c_char,
    gl_offs: usize,
    gl_flags: c_int,
    gl_closedir: *mut std::ffi::c_void,
    gl_readdir: *mut std::ffi::c_void,
    gl_opendir: *mut std::ffi::c_void,
    gl_lstat: *mut std::ffi::c_void,
    gl_stat: *mut std::ffi::c_void,
}

unsafe extern "C" {
    fn glob(pattern: *const c_char, flags: c_int, errfunc: *const std::ffi::c_void, pglob: *mut GlobT) -> c_int;
    fn globfree(pglob: *mut GlobT);
}

const GLOB_MARK: c_int = 1 << 1;
const GLOB_APPEND: c_int = 1 << 5;
const GLOB_NOSPACE: c_int = 1;
const GLOB_ABORTED: c_int = 2;

// R: sysutils.c:do_glob
fn do_glob(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let Value::Str(x) = arg(&args, 0) else {
        return Err(Flow::error("invalid 'paths' argument"));
    };
    if x.data.is_empty() {
        return Ok(Value::strs(vec![]));
    }
    let dirmark = as_logical(&arg(&args, 1));
    if dirmark == NA_LOGICAL {
        return Err(Flow::error("invalid 'dirmark' argument"));
    }
    // SAFETY: an all-zero glob_t is its documented initial state; glob()
    // fills it and globfree() releases it.
    let mut g: GlobT = unsafe { std::mem::zeroed() };
    let mut initialized = false;
    for el in x.data.iter() {
        let Some(el) = el else { continue };
        let Ok(pat) = CString::new(el.as_bytes()) else { continue };
        let flags = (if dirmark != 0 { GLOB_MARK } else { 0 }) | (if initialized { GLOB_APPEND } else { 0 });
        let res = unsafe { glob(pat.as_ptr(), flags, std::ptr::null(), &mut g) };
        if res == GLOB_ABORTED {
            i.warning(format!("read error on '{}'", el))?;
        }
        if res == GLOB_NOSPACE {
            return Err(Flow::error("internal out-of-memory condition"));
        }
        initialized = true;
    }
    let mut out = Vec::new();
    if initialized {
        for k in 0..g.gl_pathc {
            // SAFETY: glob() returned gl_pathc valid C strings.
            let p = unsafe { CStr::from_ptr(*g.gl_pathv.add(k)) };
            out.push(Some(std::rc::Rc::from(p.to_string_lossy().as_ref())));
        }
        unsafe { globfree(&mut g) };
    }
    Ok(Value::strs(out))
}

// R: Renviron.c:do_readEnviron
fn do_read_renviron(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let fname = match args.first().map(|a| &a.value) {
        Some(Value::Str(s)) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("argument 'x' must be a character string")),
    };
    let fname = super::io::expand_file_name(&fname);
    match crate::startup::process_renviron(&fname) {
        None => {
            i.warning(format!("file '{}' cannot be opened for reading", fname))?;
            Ok(Value::lgl(false))
        }
        Some(invalid) => {
            if let Some(msg) = invalid {
                i.warningcall(&Value::Null, msg)?;
            }
            Ok(Value::lgl(true))
        }
    }
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
    if p.is_null() { None } else { Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()) }
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

/// `commandArgs()`: the arguments of the R process, the way the front
/// end passed them (R: CommandLineArgs.c:do_commandArgs over the copy
/// `R_set_command_line_arguments` made). The command line front end
/// computes them; without one (`--no-base`), reconstruct Rscript's.
fn do_command_args(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    if let Some(args) = &i.cli_args {
        return Ok(Value::strs_from(args.clone()));
    }
    let argv: Vec<String> = std::env::args().collect();
    let mut out: Vec<String> = Vec::new();
    out.push(argv.first().cloned().unwrap_or_else(|| "robigo".into()));
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
        _ => Err(Flow::error("robigo: R.home() is not available: R_HOME is not set")),
    }
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
        let all: Vec<String> =
            std::env::vars_os().map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy())).collect();
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
            // SAFETY: robigo is single-threaded.
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
            // SAFETY: robigo is single-threaded.
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
        if p.is_null() { "unknown".into() } else { cstr(unsafe { (*p).pw_name }).unwrap_or_else(|| "unknown".into()) }
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

/// Whether string collation must use the C locale's byte order. GNU R's
/// `Scollate` (util.c) uses `strcmp` when the collation locale is "C" and
/// the ICU root collator otherwise.
///
/// The answer is cached until `Sys.setlocale` changes the locale, as GNU R
/// caches its collator (R: util.c:Scollate's `collationLocaleSet`, reset by
/// platform.c:do_setlocale through `resetICUcollator`): asking the C
/// library on every comparison made sorting strings several times slower.
#[inline]
pub(crate) fn collation_is_c() -> bool {
    match COLLATION_IS_C.get() {
        Some(c) => c,
        None => collation_is_c_slow(),
    }
}

#[cold]
fn collation_is_c_slow() -> bool {
    init_locale();
    let c = match cstr(unsafe { setlocale(LC_COLLATE, std::ptr::null()) }) {
        Some(l) => l == "C",
        None => true,
    };
    COLLATION_IS_C.set(Some(c));
    c
}

thread_local! {
    static COLLATION_IS_C: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
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
    // R: platform.c:do_setlocale calls resetICUcollator for LC_ALL and
    // LC_COLLATE; dropping the cache for every category is equivalent.
    COLLATION_IS_C.set(None);
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
            let (neg, set) =
                if set.first() == Some(&'!') || set.first() == Some(&'^') { (true, &set[1..]) } else { (false, set) };
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
    if cur.is_empty() { vec![pattern.to_string()] } else { cur }
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

// ----------------------------------------------------------------------
// list.files / list.dirs

/// R's `R_PATH_MAX` (Unix `PATH_MAX`); `path_buffer_append` warns when a
/// path plus its terminator grows past it.
const R_PATH_MAX: usize = 4096;

/// `R_IsDirPath` (connections.c): `stat` follows symlinks.
fn is_dir_path(path: &str) -> bool {
    std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

/// glibc's `struct dirent` (Linux LP64: `d_name` at byte offset 19).
#[repr(C)]
struct Dirent {
    d_ino: u64,
    d_off: u64,
    d_reclen: u16,
    d_type: u8,
    d_name: [u8; 256],
}

unsafe extern "C" {
    fn opendir(name: *const c_char) -> *mut std::ffi::c_void;
    fn readdir(dirp: *mut std::ffi::c_void) -> *mut Dirent;
    fn closedir(dirp: *mut std::ffi::c_void) -> c_int;
}

/// The entry names of a directory in readdir order, `"."` and `".."`
/// included (R: platform.c R_readdir; `std::fs::read_dir` hides them).
/// `None` when the path is not an openable directory.
fn read_dir_raw(path: &str) -> Option<Vec<String>> {
    let c = CString::new(path.as_bytes()).ok()?;
    // SAFETY: opendir/readdir/closedir are called with the returned
    // handle only, single-threaded; d_name is NUL-terminated by readdir.
    let dir = unsafe { opendir(c.as_ptr()) };
    if dir.is_null() {
        return None;
    }
    let mut names = Vec::new();
    loop {
        let de = unsafe { readdir(dir) };
        if de.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr((*de).d_name.as_ptr().cast()) };
        names.push(name.to_string_lossy().into_owned());
    }
    unsafe { closedir(dir) };
    Some(names)
}

/// `path_buffer_append`'s "over-long path" warning (Unix).
fn warn_over_long(i: &mut Interp, newlen: usize) -> RResult<()> {
    if newlen > R_PATH_MAX {
        i.warning("over-long path")?;
    }
    Ok(())
}

/// `list_files`' recursion: `dir` is the parent path ending in a
/// separator, `offset` the byte where an entry's name starts in a full
/// path (0 for `full.names`, else the name alone).
// R: platform.c list_files
#[allow(clippy::too_many_arguments)]
fn list_files(
    i: &mut Interp,
    names: &[String],
    dir: &str,
    offset: usize,
    out: &mut Vec<String>,
    reg: Option<&super::grep::TreBoolMatch>,
    allfiles: bool,
    recursive: bool,
    idirs: bool,
    allowdots: bool,
) -> RResult<()> {
    for name in names {
        if !allfiles && name.starts_with('.') {
            continue; // R_HiddenFile
        }
        let full = format!("{}{}", dir, name);
        warn_over_long(i, full.len() + 1)?;
        let not_dot = name != "." && name != "..";
        if recursive {
            if is_dir_path(&full) {
                if not_dot {
                    if idirs && reg.is_none_or(|r| r.is_match(name)) {
                        out.push(full[offset..].to_string());
                    }
                    // Recurse when the directory can be opened; its
                    // children append to the entry path plus a separator.
                    if let Some(children) = read_dir_raw(&full) {
                        list_files(
                            i,
                            &children,
                            &format!("{}/", full),
                            offset,
                            out,
                            reg,
                            allfiles,
                            recursive,
                            idirs,
                            allowdots,
                        )?;
                    }
                }
                continue;
            }
        }
        if (not_dot || allowdots) && reg.is_none_or(|r| r.is_match(name)) {
            out.push(full[offset..].to_string());
        }
    }
    Ok(())
}

/// `.Internal(list.files(path, pattern, all.files, full.names, recursive,
/// ignore.case, include.dirs, no..))`
// R: platform.c do_listfiles
fn do_listfiles(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let d = match arg(&args, 0) {
        Value::Str(s) => s.data.clone(),
        _ => return Err(Flow::error("invalid 'path' argument")),
    };
    let p = arg(&args, 1);
    let mut pattern: Option<String> = None;
    match &p {
        Value::Null => {}
        Value::Str(s) if !s.data.is_empty() && s.data[0].is_some() => {
            pattern = Some(s.data[0].as_deref().unwrap().to_string());
        }
        Value::Str(s) if s.data.is_empty() => {}
        _ => return Err(Flow::error("invalid 'pattern' argument")),
    }
    let allfiles = super::character::as_bool2(&arg(&args, 2), call)?;
    let fullnames = as_logical(&arg(&args, 3));
    if fullnames == NA_LOGICAL {
        return Err(Flow::error("invalid 'full.names' argument"));
    }
    let fullnames = fullnames != 0;
    let recursive = super::character::as_bool2(&arg(&args, 4), call)?;
    let igcase = as_logical(&arg(&args, 5));
    if igcase == NA_LOGICAL {
        return Err(Flow::error("invalid 'ignore.case' argument"));
    }
    let idirs = super::character::as_bool2(&arg(&args, 6), call)?;
    let nodots = as_logical(&arg(&args, 7));
    if nodots == NA_LOGICAL {
        return Err(Flow::error("invalid 'no..' argument"));
    }
    let allowdots = nodots == 0;
    let reg = match &pattern {
        Some(p) => Some(
            super::grep::TreBoolMatch::compile(p, igcase != 0)
                .map_err(|_| Flow::error("invalid 'pattern' regular expression"))?,
        ),
        None => None,
    };
    let mut out: Vec<String> = Vec::new();
    for path in d.iter().flatten() {
        let expanded = expand_file_name(path);
        // search_setup: skip paths that do not name an openable directory.
        let Some(names) = read_dir_raw(&expanded) else { continue };
        // Unix search_setup always appends the separator.
        let dir = format!("{}/", expanded);
        let offset = if fullnames { 0 } else { dir.len() };
        list_files(i, &names, &dir, offset, &mut out, reg.as_ref(), allfiles, recursive, idirs, allowdots)?;
    }
    out.sort_by(|a, b| super::sort::scollate(a, b));
    Ok(Value::strs_from(out))
}

/// `list_dirs`' recursion.
// R: platform.c list_dirs
fn list_dirs(
    i: &mut Interp,
    names: &[String],
    dir: &str,
    offset: usize,
    out: &mut Vec<String>,
    recursive: bool,
) -> RResult<()> {
    for name in names {
        let full = format!("{}{}", dir, name);
        warn_over_long(i, full.len() + 1)?;
        if is_dir_path(&full) && name != "." && name != ".." {
            out.push(full[offset..].to_string());
            if recursive {
                if let Some(children) = read_dir_raw(&full) {
                    list_dirs(i, &children, &format!("{}/", full), offset, out, recursive)?;
                }
            }
        }
    }
    Ok(())
}

/// `.Internal(list.dirs(path, full.names, recursive))`
// R: platform.c do_listdirs
fn do_listdirs(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let d = match arg(&args, 0) {
        Value::Str(s) => s.data.clone(),
        _ => return Err(Flow::error("invalid 'directory' argument")),
    };
    let fullnames = as_logical(&arg(&args, 1));
    if fullnames == NA_LOGICAL {
        return Err(Flow::error("invalid 'full.names' argument"));
    }
    let fullnames = fullnames != 0;
    let recursive = super::character::as_bool2(&arg(&args, 2), call)?;
    let mut out: Vec<String> = Vec::new();
    for path in d.iter().flatten() {
        let expanded = expand_file_name(path);
        let Some(names) = read_dir_raw(&expanded) else { continue };
        let dir = format!("{}/", expanded);
        let offset = if fullnames { 0 } else { dir.len() };
        // list.dirs(recursive=TRUE) includes the initial directory itself
        // (the expanded path without the separator search_setup added).
        if recursive {
            out.push(if fullnames { expanded.clone() } else { String::new() });
        }
        list_dirs(i, &names, &dir, offset, &mut out, recursive)?;
    }
    out.sort_by(|a, b| super::sort::scollate(a, b));
    Ok(Value::strs_from(out))
}

// ----------------------------------------------------------------------
// Files: access, info, copy, chmod, readlink; localeconv

/// `checkArity` for a `.Internal`: GNU R's `checkArity` reports through
/// `error()`, i.e. against the context call, not `errorcall(call, ...)`.
// R: util.c:Rf_checkArityCall
fn check_internal_arity(args: &[Arg], n: usize, name: &str) -> RResult<()> {
    if args.len() != n {
        return Err(Flow::error(format!(
            "{} argument{} passed to .Internal({}) which requires {}",
            args.len(),
            if args.len() == 1 { "" } else { "s" },
            name,
            n
        )));
    }
    Ok(())
}

// do_copy refuses paths past R_PATH_MAX ("over-long path"); the constant
// is defined above with list.files.
// R: platform.c:APPENDBUFSIZE
const APPEND_BUFSIZE: usize = 128 * 1024;

unsafe extern "C" {
    fn access(path: *const c_char, mode: c_int) -> c_int;
    fn chmod(path: *const c_char, mode: u32) -> c_int;
    fn umask(mask: u32) -> u32;
    fn readlink(path: *const c_char, buf: *mut c_char, bufsiz: usize) -> isize;
    fn utimensat(dirfd: c_int, path: *const c_char, times: *const [Timespec; 2], flags: c_int) -> c_int;
    fn localeconv() -> *const Lconv;
    fn getgrgid(gid: u32) -> *const Group;
}

#[repr(C)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[repr(C)]
struct Group {
    gr_name: *const c_char,
    gr_passwd: *const c_char,
    gr_gid: u32,
    gr_mem: *const *const c_char,
}

#[repr(C)]
struct Lconv {
    decimal_point: *const c_char,
    thousands_sep: *const c_char,
    grouping: *const c_char,
    int_curr_symbol: *const c_char,
    currency_symbol: *const c_char,
    mon_decimal_point: *const c_char,
    mon_thousands_sep: *const c_char,
    mon_grouping: *const c_char,
    positive_sign: *const c_char,
    negative_sign: *const c_char,
    int_frac_digits: c_char,
    frac_digits: c_char,
    p_cs_precedes: c_char,
    p_sep_by_space: c_char,
    n_cs_precedes: c_char,
    n_sep_by_space: c_char,
    p_sign_posn: c_char,
    n_sign_posn: c_char,
    int_p_cs_precedes: c_char,
    int_p_sep_by_space: c_char,
    int_n_cs_precedes: c_char,
    int_n_sep_by_space: c_char,
    int_p_sign_posn: c_char,
    int_n_sign_posn: c_char,
}

fn cstr_or_empty(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

// R: platform.c:do_localeconv
fn do_localeconv(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_internal_arity(&args, 0, "Sys.localeconv")?;
    let lc = unsafe { localeconv() };
    if lc.is_null() {
        return Err(Flow::error("localeconv failed"));
    }
    let lc = unsafe { &*lc };
    let mut names: Vec<&str> = Vec::with_capacity(18);
    let mut vals: Vec<String> = Vec::with_capacity(18);
    macro_rules! num {
        ($field:ident, $name:expr) => {
            names.push($name);
            vals.push(format!("{}", lc.$field as i32));
        };
    }
    names.push("decimal_point");
    vals.push(cstr_or_empty(lc.decimal_point));
    names.push("thousands_sep");
    vals.push(cstr_or_empty(lc.thousands_sep));
    names.push("grouping");
    vals.push(cstr_or_empty(lc.grouping));
    names.push("int_curr_symbol");
    vals.push(cstr_or_empty(lc.int_curr_symbol));
    names.push("currency_symbol");
    vals.push(cstr_or_empty(lc.currency_symbol));
    names.push("mon_decimal_point");
    vals.push(cstr_or_empty(lc.mon_decimal_point));
    names.push("mon_thousands_sep");
    vals.push(cstr_or_empty(lc.mon_thousands_sep));
    names.push("mon_grouping");
    vals.push(cstr_or_empty(lc.mon_grouping));
    names.push("positive_sign");
    vals.push(cstr_or_empty(lc.positive_sign));
    names.push("negative_sign");
    vals.push(cstr_or_empty(lc.negative_sign));
    num!(int_frac_digits, "int_frac_digits");
    num!(frac_digits, "frac_digits");
    num!(p_cs_precedes, "p_cs_precedes");
    num!(p_sep_by_space, "p_sep_by_space");
    num!(n_cs_precedes, "n_cs_precedes");
    num!(n_sep_by_space, "n_sep_by_space");
    num!(p_sign_posn, "p_sign_posn");
    num!(n_sign_posn, "n_sign_posn");
    let mut v = Value::strs_from(vals);
    v.set_attr_raw(s::NAMES, Value::strs_from(names));
    Ok(v)
}

// R: platform.c:do_fileaccess
fn do_fileaccess(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_internal_arity(&args, 2, "file.access")?;
    let Value::Str(files) = &args[0].value else { return Err(Flow::error("invalid 'names' argument")) };
    let mode = as_integer(&args[1].value);
    if !(0..=7).contains(&mode) {
        return Err(Flow::error("invalid 'mode' argument"));
    }
    // mode bit 1 = X_OK, 2 = W_OK, 4 = R_OK.
    let modemask = ((mode & 1 != 0) as c_int) * 1 | ((mode & 2 != 0) as c_int) * 2 | ((mode & 4 != 0) as c_int) * 4;
    let mut out = Vec::with_capacity(files.data.len());
    for f in &files.data {
        out.push(match f {
            // R treats NA as a non-existent file.
            None => -1,
            Some(f) => {
                let path = expand_file_name(f);
                match CString::new(path) {
                    Ok(c) => unsafe { access(c.as_ptr(), modemask) },
                    Err(_) => -1,
                }
            }
        });
    }
    Ok(Value::ints(out))
}

/// `getpwuid`, as R uses it for `file.info(extra_cols = TRUE)`.
fn user_name(uid: u32) -> RStr {
    // `getpwuid` is declared above (pw_name is its first member).
    let p = unsafe { getpwuid(uid) };
    if p.is_null() { None } else { Some(cstr_or_empty(unsafe { &*p }.pw_name).into()) }
}

fn group_name(gid: u32) -> RStr {
    let p = unsafe { getgrgid(gid) };
    if p.is_null() { None } else { Some(cstr_or_empty(unsafe { &*p }.gr_name).into()) }
}

// R: platform.c:do_fileinfo
fn do_fileinfo(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    use std::os::unix::fs::MetadataExt;
    check_internal_arity(&args, 2, "file.info")?;
    let Value::Str(files) = &args[0].value else { return Err(Flow::error("invalid filename argument")) };
    let extras = as_integer(&args[1].value);
    if extras == NA_INTEGER {
        return Err(Flow::error("invalid 'extra_cols' argument"));
    }
    let extras = extras != 0;
    let n = files.data.len();
    let mut size = vec![NA_REAL; n];
    let mut isdir = vec![NA_INTEGER; n];
    let mut mode = vec![NA_INTEGER; n];
    let mut mtime = vec![NA_REAL; n];
    let mut ctime = vec![NA_REAL; n];
    let mut atime = vec![NA_REAL; n];
    let mut uid = vec![NA_INTEGER; n];
    let mut gid = vec![NA_INTEGER; n];
    let mut unames: Vec<RStr> = vec![None; n];
    let mut grnames: Vec<RStr> = vec![None; n];
    let ts = |sec: i64, nsec: i64| sec as f64 + 1e-9 * nsec as f64;
    for (k, f) in files.data.iter().enumerate() {
        let Some(f) = f else { continue };
        // R: "Target not link" -- stat(), not lstat().
        let Ok(md) = std::fs::metadata(expand_file_name(f)) else { continue };
        size[k] = md.size() as f64;
        isdir[k] = md.is_dir() as i32;
        mode[k] = (md.mode() & 0o7777) as i32;
        mtime[k] = ts(md.mtime(), md.mtime_nsec());
        ctime[k] = ts(md.ctime(), md.ctime_nsec());
        atime[k] = ts(md.atime(), md.atime_nsec());
        uid[k] = md.uid() as i32;
        gid[k] = md.gid() as i32;
        unames[k] = user_name(md.uid());
        grnames[k] = group_name(md.gid());
    }
    // `mode` carries class "octmode" (R: platform.c:classgets(mode, "octmode")).
    let mut mode_v = Value::ints(mode);
    mode_v.set_attr_raw(s::CLASS, Value::str("octmode"));
    let mut cols = vec![
        Value::reals(size),
        Value::lgls(isdir),
        mode_v,
        Value::reals(mtime),
        Value::reals(ctime),
        Value::reals(atime),
    ];
    let mut names = vec!["size", "isdir", "mode", "mtime", "ctime", "atime"];
    if extras {
        cols.push(Value::ints(uid));
        cols.push(Value::ints(gid));
        cols.push(Value::strs(unames));
        cols.push(Value::strs(grnames));
        names.extend(["uid", "gid", "uname", "grname"]);
    }
    let mut ans = Value::list(cols);
    ans.set_attr_raw(s::NAMES, Value::strs_from(names));
    Ok(ans)
}

/// The permission mask `do_copy` applies: `0777 & ~umask`.
fn umask_mask() -> u32 {
    let um = unsafe { umask(0) };
    unsafe { umask(um) };
    0o777 & !um
}

fn chmod_path(path: &str, mode: u32) -> bool {
    let Ok(c) = CString::new(path) else { return false };
    (unsafe { chmod(c.as_ptr(), mode) }) == 0
}

// R: platform.c:copyFileTime
fn copy_file_time(from: &str, to: &str) {
    use std::os::unix::fs::MetadataExt;
    let Ok(md) = std::fs::metadata(expand_file_name(from)) else { return };
    let ftime = md.mtime() as f64 + 1e-9 * md.mtime_nsec() as f64;
    let sec = ftime as i32 as i64;
    let nsec = (1e9 * (ftime - ftime as i32 as f64)) as i64;
    let times = [Timespec { tv_sec: sec, tv_nsec: nsec }, Timespec { tv_sec: sec, tv_nsec: nsec }];
    if let Ok(c) = CString::new(to) {
        unsafe { utimensat(-100, c.as_ptr(), &times, 0) };
    }
}

/// Copy `from` + `name` to `to` + `name`, recursively for directories.
/// Returns the number of failures (0 on success).
// R: platform.c:do_copy
#[allow(clippy::too_many_arguments)]
fn do_copy(
    i: &mut Interp,
    from: &str,
    name: &str,
    to: &str,
    over: bool,
    recursive: bool,
    perms: bool,
    dates: bool,
    depth: i32,
) -> RResult<i32> {
    if depth > 100 {
        i.warning("too deep nesting")?;
        return Ok(1);
    }
    use std::os::unix::fs::MetadataExt;
    let this = format!("{}{}", from, name);
    if this.len() >= R_PATH_MAX {
        i.warning("over-long path")?;
        return Ok(1);
    }
    let mask = umask_mask();
    let md = std::fs::symlink_metadata(&this).ok();
    let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    let mut nfail = 0;
    if is_dir {
        if !recursive {
            return Ok(1);
        }
        let dest = format!("{}{}", to, name);
        if dest.len() >= R_PATH_MAX {
            i.warning("over-long path")?;
            return Ok(1);
        }
        // R calls mkdir(dest, 0700) and only later chmods the final mode.
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(&dest) {
            Ok(()) => {}
            Err(e) => {
                if e.kind() != std::io::ErrorKind::AlreadyExists {
                    i.warning(format!("problem creating directory {}: {}", this, os_error_message(&e)))?;
                    return Ok(1);
                }
                match std::fs::metadata(&dest) {
                    Ok(d) if over && !d.is_dir() => {
                        i.warning(format!("cannot overwrite non-directory {} with directory {}", dest, this))?;
                        return Ok(1);
                    }
                    _ => {}
                }
            }
        }
        let dest_slash = format!("{}/", dest);
        match std::fs::read_dir(&this) {
            Ok(entries) => {
                for e in entries {
                    let Ok(e) = e else { continue };
                    let dname = e.file_name().to_string_lossy().into_owned();
                    if dname == "." || dname == ".." {
                        continue;
                    }
                    let p = format!("{}/{}", name, dname);
                    nfail += do_copy(i, from, &p, to, over, recursive, perms, dates, depth + 1)?;
                }
            }
            Err(e) => {
                i.warning(format!("problem reading directory {}: {}", this, os_error_message(&e)))?;
                nfail += 1;
            }
        }
        let mode = if perms { (md.as_ref().map(|m| m.mode()).unwrap_or(0o777) & mask) & 0o7777 } else { mask };
        chmod_path(&dest_slash, mode);
        if dates {
            copy_file_time(&this, &dest_slash);
        }
    } else {
        let dest = format!("{}{}", to, name);
        if dest.len() >= R_PATH_MAX {
            i.warning("over-long path")?;
            return Ok(1);
        }
        if over || std::fs::metadata(&dest).is_err() {
            match std::fs::read(&this) {
                Ok(bytes) => {
                    if let Err(e) = std::fs::write(&dest, &bytes) {
                        i.warning(format!("problem copying {} to {}: {}", this, dest, os_error_message(&e)))?;
                        return Ok(1);
                    }
                    // R copies with fread/fwrite in APPENDBUFSIZE chunks; the
                    // result is the same bytes.
                    let _ = APPEND_BUFSIZE;
                }
                Err(e) => {
                    i.warning(format!("problem copying {} to {}: {}", this, dest, os_error_message(&e)))?;
                    return Ok(1);
                }
            }
        } else {
            // !over and the destination exists.
            return Ok(1);
        }
        if perms {
            let mode = (md.as_ref().map(|m| m.mode()).unwrap_or(0o777) & mask) & 0o7777;
            chmod_path(&dest, mode);
        }
        if dates {
            copy_file_time(&this, &dest);
        }
    }
    Ok(nfail)
}

// R: platform.c:do_filecopy
fn do_filecopy(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_internal_arity(&args, 6, "file.copy")?;
    let nfiles = match &args[0].value {
        Value::Str(s) => s.data.len(),
        Value::Null => 0,
        _ => 0,
    };
    if nfiles == 0 {
        return Ok(Value::lgls(vec![]));
    }
    let Value::Str(files) = &args[0].value else { return Err(Flow::error("invalid 'from' argument")) };
    let Value::Str(to) = &args[1].value else { return Err(Flow::error("invalid 'to' argument")) };
    if to.data.len() != 1 {
        return Err(Flow::error("invalid 'to' argument"));
    }
    let over = as_logical(&args[2].value);
    if over == NA_LOGICAL {
        return Err(Flow::error("invalid 'overwrite' argument"));
    }
    let recursive = as_logical(&args[3].value);
    if recursive == NA_LOGICAL {
        return Err(Flow::error("invalid 'recursive' argument"));
    }
    let perms = as_logical(&args[4].value);
    if perms == NA_LOGICAL {
        return Err(Flow::error("invalid 'copy.mode' argument"));
    }
    let dates = as_logical(&args[5].value);
    if dates == NA_LOGICAL {
        return Err(Flow::error("invalid 'copy.date' argument"));
    }
    let (over, recursive, perms, dates) = (over != 0, recursive != 0, perms != 0, dates != 0);
    let to = to.data[0].as_deref().unwrap_or("NA");
    let q = expand_file_name(to);
    if q.len() > R_PATH_MAX - 2 {
        return Err(Flow::error("invalid 'to' argument"));
    }
    let dir = if q.ends_with('/') { q } else { format!("{}/", q) };
    let mut ans: Vec<i32> = Vec::with_capacity(files.data.len());
    for f in &files.data {
        let Some(f) = f else {
            ans.push(FALSE);
            continue;
        };
        let mut from = expand_file_name(f);
        if from.is_empty() {
            ans.push(FALSE);
            continue;
        }
        // A trailing separator is a mistake.
        if from.ends_with('/') {
            from.pop();
        }
        let (srcdir, name) = match from.rfind('/') {
            Some(k) => (from[..=k].to_string(), from[k + 1..].to_string()),
            None => ("./".to_string(), from.clone()),
        };
        ans.push((do_copy(i, &srcdir, &name, &dir, over, recursive, perms, dates, 1)? == 0) as i32);
    }
    Ok(Value::lgls(ans))
}

// R: platform.c:do_syschmod
fn do_syschmod(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_internal_arity(&args, 3, "Sys.chmod")?;
    let Value::Str(paths) = &args[0].value else { return Err(Flow::error("invalid 'paths' argument")) };
    let modes = crate::coerce::to_int(&args[1].value, &mut Default::default());
    let n = paths.data.len();
    let m = modes.len();
    if m == 0 && n > 0 {
        return Err(Flow::error("'mode' must be of length at least one"));
    }
    let use_umask = as_logical(&args[2].value);
    if use_umask == NA_LOGICAL {
        return Err(Flow::error("invalid 'use_umask' argument"));
    }
    let um = unsafe { umask(0) };
    unsafe { umask(um) };
    let mut out = Vec::with_capacity(n);
    for (k, p) in paths.data.iter().enumerate() {
        let mut mode = modes[k % m] as u32;
        if mode == NA_INTEGER as u32 {
            mode = 0o777;
        }
        if use_umask != 0 {
            mode &= !um;
        }
        let ok = match p {
            Some(p) => chmod_path(&expand_file_name(p), mode),
            None => false,
        };
        out.push(ok as i32);
    }
    Ok(Value::lgls(out))
}

// R: platform.c:do_readlink
fn do_readlink(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_internal_arity(&args, 1, "Sys.readlink")?;
    let Value::Str(paths) = &args[0].value else { return Err(Flow::error("invalid 'paths' argument")) };
    let mut out: Vec<RStr> = Vec::with_capacity(paths.data.len());
    for p in &paths.data {
        let Some(p) = p else {
            out.push(None);
            continue;
        };
        let path = expand_file_name(p);
        let Ok(c) = CString::new(path) else {
            out.push(None);
            continue;
        };
        let mut buf = vec![0u8; R_PATH_MAX + 1];
        let res = unsafe { readlink(c.as_ptr(), buf.as_mut_ptr() as *mut c_char, R_PATH_MAX) };
        if res as usize == R_PATH_MAX {
            let s = String::from_utf8_lossy(&buf[..res as usize]).into_owned();
            i.warning(format!("possible truncation of value for element {}", out.len() + 1))?;
            out.push(Some(s.into()));
        } else if res >= 0 {
            let s = String::from_utf8_lossy(&buf[..res as usize]).into_owned();
            out.push(Some(s.into()));
        } else if std::io::Error::last_os_error().raw_os_error() == Some(22) {
            // EINVAL: not a symbolic link.
            out.push(Some("".into()));
        } else {
            out.push(None);
        }
    }
    Ok(Value::strs(out))
}

// R: platform.c:do_setFileTime (utimensat)
fn do_setfiletime(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let paths = match arg(&args, 0) {
        Value::Str(p) => p,
        _ => return Err(Flow::error("invalid 'path' argument")),
    };
    let times = match i.coerce(&arg(&args, 1), VType::Real)? {
        Value::Real(t) => t.data.to_vec(),
        _ => Vec::new(),
    };
    let n = paths.data.len();
    if times.is_empty() && n > 0 {
        return Err(Flow::error("'time' must be of length at least one"));
    }
    let mut ans = Vec::with_capacity(n);
    for (k, p) in paths.data.iter().enumerate() {
        let ftime = times[k % times.len()];
        let secs = ftime as i32;
        let nsec = (1e9 * (ftime - secs as f64)) as i32;
        let ts = || Timespec { tv_sec: secs as i64, tv_nsec: nsec as i64 };
        let fname = CString::new(p.as_deref().unwrap_or("NA")).unwrap_or_default();
        // AT_FDCWD
        let res = unsafe { utimensat(-100, fname.as_ptr(), &[ts(), ts()], 0) } == 0;
        ans.push(res as Lgl);
    }
    Ok(Value::lgls(ans))
}

unsafe extern "C" {
    fn lzma_version_string() -> *const c_char;
    fn gnu_get_libc_version() -> *const c_char;
}

// R: platform.c:do_eSoftVersion — the versions of the external libraries
// in use; "" for those robigo does not use (it has its own deflate,
// regular-expression, LAPACK code and no bzip2, ICU or readline)
fn do_esoftversion(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let c = |p: *const c_char| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    let xz = c(unsafe { lzma_version_string() });
    let iconv = format!("glibc {}", c(unsafe { gnu_get_libc_version() }));
    let vals = ["", "", &xz, "", "", "", "", &iconv, "", ""];
    let names = ["zlib", "bzlib", "xz", "libdeflate", "PCRE", "ICU", "TRE", "iconv", "readline", "BLAS"];
    let mut ans = Value::strs_from(vals);
    ans.set_attr_raw(s::NAMES, Value::strs_from(names));
    Ok(ans)
}
