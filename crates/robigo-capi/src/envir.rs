//! envir.c, eval.c, context.c (R_tryEval), parse.c and print.c entry
//! points. Until G3 environments live in the host runtime, so these are
//! host operations (`docs/core.md` §6).

use crate::host::{op, try_op};
use crate::memory::*;
use crate::sort::internal;
use crate::state::*;
use robigo_core::Sexp;
use robigo_core::types::*;
use std::ffi::{CStr, c_char, c_int};

fn lgl(b: bool) -> Sexp {
    unsafe { crate::list::Rf_ScalarLogical(b as c_int) }
}

unsafe fn int(x: c_int) -> Sexp {
    crate::list::Rf_ScalarInteger(x)
}

// R: eval.c:eval
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_eval(e: Sexp, rho: Sexp) -> Sexp {
    let mut out = Sexp::NULL;
    let t = (crate::host::host().eval)(e, rho, &mut out);
    if t != 0 {
        crate::errors::jump_host(t)
    }
    out
}

/// R_tryEval: evaluate at top level (outer handlers hidden, errors
/// reported by the default handler), never jump.
unsafe fn try_eval(e: Sexp, env: Sexp, err: *mut c_int) -> Sexp {
    let mut tmp = Sexp::NULL;
    try_op("toplevelBegin\0", &[], &mut tmp);
    let mut out = Sexp::NULL;
    let t = (crate::host::host().eval)(e, env, &mut out);
    try_op("toplevelEnd\0", &[], &mut tmp);
    if t != 0 {
        (crate::host::host().report)(t);
        if !err.is_null() {
            *err = 1;
        }
        return Sexp::NULL;
    }
    if !err.is_null() {
        *err = 0;
    }
    out
}

// R: context.c:R_tryEval
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_tryEval(e: Sexp, env: Sexp, err: *mut c_int) -> Sexp {
    try_eval(e, env, err)
}

// R: eval.c:R_forceAndCall — the host evaluates calls with forced
// arguments the same way
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_forceAndCall(e: Sexp, _n: c_int, rho: Sexp) -> Sexp {
    Rf_eval(e, rho)
}

// R: envir.c:R_NewEnv
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_NewEnv(enclos: Sexp, _hash: c_int, _size: c_int) -> Sexp {
    op("newEnv\0", &[enclos])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_NewEnvironment(_namelist: Sexp, _valuelist: Sexp, rho: Sexp) -> Sexp {
    op("newEnv\0", &[rho])
}

// R: envir.c:defineVar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_defineVar(sym: Sexp, val: Sexp, rho: Sexp) {
    op("defineVar\0", &[sym, val, rho]);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_setVar(sym: Sexp, val: Sexp, rho: Sexp) {
    op("setVar\0", &[sym, val, rho]);
}

// R: envir.c:findVar — the binding's value (promises unforced), through
// the enclosures; R_UnboundValue if none
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_findVar(sym: Sexp, rho: Sexp) -> Sexp {
    op("findVar\0", &[sym, rho])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_findVarInFrame(rho: Sexp, sym: Sexp) -> Sexp {
    op("findVarInFrame\0", &[rho, sym])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_findVarInFrame3(rho: Sexp, sym: Sexp, _doget: c_int) -> Sexp {
    op("findVarInFrame\0", &[rho, sym])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_existsVarInFrame(rho: Sexp, sym: Sexp) -> c_int {
    (*LOGICAL(op("existsVarInFrame\0", &[rho, sym])) == 1) as c_int
}

// R: envir.c:R_removeVarFromFrame (name, env — the other order)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_removeVarFromFrame(name: Sexp, env: Sexp) {
    op("removeVar\0", &[name, env]);
}

// R: envir.c:findFun
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_findFun(sym: Sexp, rho: Sexp) -> Sexp {
    op("findFun\0", &[sym, rho])
}

// R: envir.c:R_getVar — forces promises; "object 'x' not found"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_getVar(sym: Sexp, rho: Sexp, inherits: c_int) -> Sexp {
    op("getVar\0", &[sym, rho, lgl(inherits != 0), R_UnboundValue])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_getVarEx(sym: Sexp, rho: Sexp, inherits: c_int, ifnotfound: Sexp) -> Sexp {
    op("getVar\0", &[sym, rho, lgl(inherits != 0), ifnotfound])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ParentEnv(env: Sexp) -> Sexp {
    if env.typ() != ENVSXP {
        crate::errors::error_str("argument is not an environment")
    }
    ENCLOS(env)
}

// R: envir.c:R_lsInternal3
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_lsInternal3(env: Sexp, all: c_int, sorted: c_int) -> Sexp {
    internal("ls", &[env, lgl(all != 0), lgl(sorted != 0)])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_lsInternal(env: Sexp, all: c_int) -> Sexp {
    R_lsInternal3(env, all, 1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_MakeActiveBinding(sym: Sexp, fun: Sexp, env: Sexp) {
    internal("makeActiveBinding", &[sym, fun, env]);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_BindingIsActive(sym: Sexp, env: Sexp) -> c_int {
    (*LOGICAL(internal("bindingIsActive", &[sym, env])) == 1) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_BindingIsLocked(sym: Sexp, env: Sexp) -> c_int {
    (*LOGICAL(internal("bindingIsLocked", &[sym, env])) == 1) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ActiveBindingFunction(sym: Sexp, env: Sexp) -> Sexp {
    internal("activeBindingFunction", &[sym, env])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_LockBinding(sym: Sexp, env: Sexp) {
    internal("lockBinding", &[sym, env]);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_unLockBinding(sym: Sexp, env: Sexp) {
    internal("unlockBinding", &[sym, env]);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_IsNamespaceEnv(rho: Sexp) -> c_int {
    (*LOGICAL(internal("isNamespaceEnv", &[rho])) == 1) as c_int
}

// R: envir.c:R_FindNamespace — calls getNamespace(info) in base
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_FindNamespace(info: Sexp) -> Sexp {
    let f = core().install(b"getNamespace");
    let call = Rf_protect(crate::list::Rf_lang2(f, info));
    let r = Rf_eval(call, R_GlobalEnv);
    Rf_unprotect(1);
    r
}

// R: unique.c:match
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_match(table: Sexp, x: Sexp, nomatch: c_int) -> Sexp {
    internal("match", &[x, table, int(nomatch), R_NilValue])
}

// ---- parsing and printing -------------------------------------------------

/// R_ParseVector's status codes (R_ext/Parse.h)
const PARSE_OK: c_int = 1;
const PARSE_ERROR: c_int = 3;

// R: parse.c:R_ParseVector
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ParseVector(text: Sexp, _n: c_int, status: *mut c_int, _srcfile: Sexp) -> Sexp {
    let mut out = Sexp::NULL;
    let t = try_op("parse\0", &[text], &mut out);
    if t != 0 {
        (crate::host::host().discard)(t);
        *status = PARSE_ERROR;
        return R_NilValue;
    }
    *status = PARSE_OK;
    out
}

// R: print.c:PrintValue
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_PrintValue(x: Sexp) {
    op("printValue\0", &[x]);
}

// R: print.c:PrintDefaults resets the print options; robigo's printer
// reads them from options() at each print
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_PrintDefaults() {}

// R: sysutils.c:R_ExpandFileName (a static buffer, as GNU R's)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ExpandFileName(s: *const c_char) -> *const c_char {
    static mut BUF: [u8; 4096] = [0; 4096];
    let name = CStr::from_ptr(s).to_string_lossy().into_owned();
    let expanded = if name == "~" || name.starts_with("~/") {
        match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => format!("{h}{}", &name[1..]),
            _ => name,
        }
    } else {
        name
    };
    let b = expanded.as_bytes();
    let n = b.len().min(4095);
    let buf = &mut *std::ptr::addr_of_mut!(BUF);
    buf[..n].copy_from_slice(&b[..n]);
    buf[n] = 0;
    buf.as_ptr() as *const c_char
}
