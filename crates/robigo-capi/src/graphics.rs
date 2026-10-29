//! What GNU R's graphics engine (graphics/, compiled into libR by
//! build.rs) needs from robigo beyond the API: the function table its
//! display-list replay calls primitives through (`PRIMFUN(op)` is
//! `R_FunTab[PRIMOFFSET(op)].cfun`), arity checks, and a few envir.c,
//! options.c and console entry points.

use std::ffi::{CStr, c_char, c_int, c_uint};

use robigo_core::Sexp;

use crate::host::op;
use crate::state::{R_BaseEnv, R_BaseNamespace, R_DevicesSymbol};

type CCode = unsafe extern "C" fn(Sexp, Sexp, Sexp, Sexp) -> Sexp;

/// GNU R's FUNTAB (Defn.h): name, C function, code, eval, arity, and the
/// deparse info (kind, precedence, rightassoc).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FunTab {
    name: *const c_char,
    cfun: Option<CCode>,
    code: c_int,
    eval: c_int,
    arity: c_int,
    kind: c_int,
    prec: c_int,
    rightassoc: c_uint,
}

const EMPTY: FunTab =
    FunTab { name: std::ptr::null(), cfun: None, code: 0, eval: 0, arity: 0, kind: 0, prec: 0, rightassoc: 0 };
const MAX_FUNTAB: usize = 4096;

/// robigo's builtins, in `all_builtins()` order; every entry calls back
/// into the runtime ([`prim_call`]).
#[unsafe(no_mangle)]
pub static mut R_FunTab: [FunTab; MAX_FUNTAB] = [EMPTY; MAX_FUNTAB];

static mut FILLED: bool = false;

unsafe fn funtab() -> &'static Vec<&'static robigo_runtime::value::Builtin> {
    static LIST: std::sync::OnceLock<Vec<&'static robigo_runtime::value::Builtin>> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(robigo_runtime::builtins::all_builtins);
    if !FILLED {
        let tab = &mut *std::ptr::addr_of_mut!(R_FunTab);
        for (k, b) in list.iter().enumerate().take(MAX_FUNTAB) {
            let name = std::ffi::CString::new(b.name).unwrap_or_default();
            tab[k] = FunTab {
                name: name.into_raw(),
                cfun: Some(prim_call),
                code: 0,
                // names.c's eval column: Y (internal) and Z (evaluated)
                eval: (b.internal as c_int) * 10
                    + matches!(b.kind, robigo_runtime::value::BuiltinKind::Builtin) as c_int,
                arity: b.arity as c_int,
                kind: 0,
                prec: 0,
                rightassoc: 0,
            };
        }
        FILLED = true;
    }
    list
}

/// A primitive called from C (`PRIMFUN(op)(call, op, args, env)`): the
/// runtime's builtin with the evaluated argument list.
unsafe extern "C" fn prim_call(call: Sexp, op_: Sexp, args: Sexp, env: Sexp) -> Sexp {
    op("applyBuiltin\0", &[call, op_, args, env])
}

unsafe fn builtin_of(x: Sexp) -> Option<&'static robigo_runtime::value::Builtin> {
    match crate::bridge::from_sexp(x) {
        robigo_runtime::value::Value::Builtin(b) => Some(b),
        _ => None,
    }
}

// R: memory.c PRIMOFFSET (robigo: the builtin's place in R_FunTab)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PRIMOFFSET(x: Sexp) -> c_int {
    let list = funtab();
    let Some(b) = builtin_of(x) else { crate::errors::error_str("robigo: PRIMOFFSET of a non-primitive") };
    match list.iter().position(|c| std::ptr::eq(*c, b)) {
        Some(k) => k as c_int,
        None => crate::errors::error_str("robigo: primitive not in the function table"),
    }
}

// R: builtin.c:Rf_checkArityCall
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_checkArityCall(op_: Sexp, args: Sexp, call: Sexp) {
    let Some(b) = builtin_of(op_) else { return };
    let n = crate::memory::Rf_length(args);
    if b.arity >= 0 && b.arity as c_int != n {
        let s = if n == 1 { "" } else { "s" };
        if b.internal {
            crate::errors::error_str_owned(format!(
                "{n} argument{s} passed to .Internal({}) which requires {}",
                b.name, b.arity
            ))
        } else {
            let msg = format!("{n} argument{s} passed to '{}' which requires {}", b.name, b.arity);
            let m = std::ffi::CString::new(msg).unwrap_or_default();
            crate::errors::raise(call, m.as_ptr())
        }
    }
}

// R: options.c:Rf_GetOptionDeviceAsk
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetOptionDeviceAsk() -> c_int {
    let sym = crate::strings::Rf_install(c"device.ask.default".as_ptr());
    let ask = crate::coerce::Rf_asLogical(crate::attrib::Rf_GetOption1(sym));
    if ask == robigo_core::NA_LOGICAL {
        crate::errors::Rf_warning(c"%s".as_ptr(), c"invalid value for \"device.ask.default\", using FALSE".as_ptr());
        return 0;
    }
    (ask != 0) as c_int
}

/// `.Devices`, the pairlist devices.c keeps current with SETCAR/SETCDR:
/// C works on this object (the bridge would hand it a copy of the base
/// binding) and the binding is refreshed from it after each native call.
static mut DEVICES: Sexp = Sexp::NULL;
static mut DEVICES_OUT: bool = false;

unsafe fn is_devices(sym: Sexp, rho: Sexp) -> bool {
    sym == R_DevicesSymbol && (rho == R_BaseEnv || rho == R_BaseNamespace)
}

/// After a native call: copy C's `.Devices` into the base binding.
pub unsafe fn sync_devices(interp: &mut robigo_runtime::interp::Interp) {
    if DEVICES_OUT {
        DEVICES_OUT = false;
        let v = crate::bridge::from_sexp(DEVICES);
        interp.base.force_set(robigo_runtime::sym::Sym::new(".Devices"), v);
    }
}

// R: envir.c:gsetVar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_gsetVar(sym: Sexp, val: Sexp, rho: Sexp) {
    if is_devices(sym, rho) {
        if !DEVICES.is_null() {
            crate::state::core().release(DEVICES);
        }
        crate::state::core().preserve(val);
        DEVICES = val;
    }
    crate::envir::Rf_defineVar(sym, val, rho)
}

// R: envir.c:R_findVar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_findVar(sym: Sexp, rho: Sexp) -> Sexp {
    if is_devices(sym, rho) {
        if DEVICES.is_null() {
            DEVICES = crate::envir::Rf_findVar(sym, rho);
            crate::state::core().preserve(DEVICES);
        }
        DEVICES_OUT = true;
        return DEVICES;
    }
    crate::envir::Rf_findVar(sym, rho)
}

// R: envir.c:R_findVarInFrame
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_findVarInFrame(rho: Sexp, sym: Sexp) -> Sexp {
    crate::envir::Rf_findVarInFrame(rho, sym)
}

// R: sys-std.c:Rstd_ReadConsole (no readline): the prompt, then a line of
// standard input; 0 at end of input
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ReadConsole(prompt: *const c_char, buf: *mut u8, len: c_int, _hist: c_int) -> c_int {
    use std::io::{BufRead, Write};
    let p = CStr::from_ptr(prompt).to_string_lossy().into_owned();
    let _ = std::io::stdout().write_all(p.as_bytes());
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => 0,
        Ok(_) => {
            let b = line.as_bytes();
            let n = b.len().min((len.max(1) - 1) as usize);
            std::ptr::copy_nonoverlapping(b.as_ptr(), buf, n);
            *buf.add(n) = 0;
            1
        }
    }
}

/// devices.c:InitGraphics: the null device, `.Device`, `.Devices`.
pub unsafe fn init_graphics() {
    Rf_InitGraphics()
}

/// R_MB_CUR_MAX after the locale is set.
pub unsafe fn locale_init() {
    robigo_locale_init()
}

unsafe extern "C" {
    fn robigo_locale_init();
    fn Rf_InitGraphics();
    pub fn do_recordGraphics(call: Sexp, op: Sexp, args: Sexp, env: Sexp) -> Sexp;
    pub fn robigo_gr_begin() -> Sexp;
    pub fn robigo_gr_end(call: Sexp, op: Sexp, args: Sexp, record: Sexp) -> Sexp;
    fn robigo_gr_roots(out: *mut Sexp, max: c_int) -> c_int;
    fn robigo_print_roots(out: *mut Sexp) -> c_int;
    fn robigo_set_OutDec(dec: *const c_char);
    fn robigo_set_max_contour_segments(k: c_int);
}

/// options(OutDec=, max.contour.segments=) set GNU R's C globals.
pub unsafe fn option_set(name: &str, value: &robigo_runtime::value::Value) {
    use robigo_runtime::value::Value;
    match (name, value) {
        ("OutDec", Value::Str(v)) => {
            let s = v.data.first().cloned().flatten().unwrap_or_default();
            let c = std::ffi::CString::new(s.as_bytes()).unwrap_or_default();
            robigo_set_OutDec(c.as_ptr())
        }
        ("max.contour.segments", Value::Int(v)) => robigo_set_max_contour_segments(v.data[0]),
        _ => {}
    }
}

/// The devices' display lists, snapshots and event environments.
pub unsafe fn mark_roots(c: &mut robigo_core::Core) {
    let mut roots = [Sexp::NULL; 3 * 64];
    let n = robigo_gr_roots(roots.as_mut_ptr(), roots.len() as c_int);
    for &x in &roots[..n as usize] {
        c.mark(x);
    }
    let n = robigo_print_roots(roots.as_mut_ptr());
    for &x in &roots[..n as usize] {
        c.mark(x);
    }
}
