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

/// R_tryEval / R_tryEvalSilent: evaluate at top level (outer handlers hidden,
/// errors reported by the default handler unless `silent`), never jump.
unsafe fn try_eval(e: Sexp, env: Sexp, err: *mut c_int, silent: bool) -> Sexp {
    let mut tmp = Sexp::NULL;
    try_op("toplevelBegin\0", &[], &mut tmp);
    let mut out = Sexp::NULL;
    let t = (crate::host::host().eval)(e, env, &mut out);
    try_op("toplevelEnd\0", &[], &mut tmp);
    if t != 0 {
        if silent {
            (crate::host::host().discard)(t);
        } else {
            (crate::host::host().report)(t);
        }
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
    try_eval(e, env, err, false)
}

// R: context.c:R_tryEvalSilent
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_tryEvalSilent(e: Sexp, env: Sexp, err: *mut c_int) -> Sexp {
    try_eval(e, env, err, true)
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

// R: envir.c:NewEnvironment — the frame is valuelist, its tags taken
// from namelist where that has elements
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_NewEnvironment(namelist: Sexp, valuelist: Sexp, rho: Sexp) -> Sexp {
    let env = Rf_protect(op("newEnv\0", &[rho]));
    let (mut v, mut n) = (valuelist, namelist);
    let mut frame = Vec::new();
    while v != R_NilValue {
        let tag = if n != R_NilValue { TAG(n) } else { TAG(v) };
        frame.push((tag, CAR(v)));
        v = CDR(v);
        if n != R_NilValue {
            n = CDR(n);
        }
    }
    // defineVar keeps the first binding of a name, as a frame lookup does
    for &(tag, val) in frame.iter().rev() {
        if tag != R_NilValue {
            Rf_defineVar(tag, val, env);
        }
    }
    Rf_unprotect(1);
    env
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

// R: envir.c:R_LockEnvironment
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_LockEnvironment(env: Sexp, bindings: c_int) {
    internal("lockEnvironment", &[env, lgl(bindings != 0)]);
}

// R: envir.c:R_PackageEnvName
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_PackageEnvName(rho: Sexp) -> Sexp {
    if !rho.is_null() && rho.typ() == ENVSXP {
        let name = crate::attrib::Rf_getAttrib(rho, R_NameSymbol);
        if name.typ() == STRSXP && LENGTH(name) > 0 && STRING_ELT(name, 0).char_bytes().starts_with(b"package:") {
            return name;
        }
    }
    R_NilValue
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_IsNamespaceEnv(rho: Sexp) -> c_int {
    (*LOGICAL(internal("isNamespaceEnv", &[rho])) == 1) as c_int
}

// R: envir.c:R_IsPackageEnv — an attached package environment, named
// "package:foo". TYPEOF gates before anything else (O(1), as in R;
// `from_sexp` would materialize a non-environment's payload).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_IsPackageEnv(rho: Sexp) -> c_int {
    if rho.typ() != ENVSXP {
        return 0;
    }
    match crate::bridge::from_sexp(rho) {
        robigo_runtime::value::Value::Env(e) => e.package_env_name().is_some() as c_int,
        _ => 0,
    }
}

// R: envir.c:R_EnvironmentIsLocked (simple_as_environment: an S4
// object's environment data slot counts as the environment)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_EnvironmentIsLocked(env: Sexp) -> c_int {
    // R checks NILSXP before anything else.
    if env == R_NilValue || env.is_null() {
        crate::errors::error_str("use of NULL environment is defunct");
    }
    // simple_as_environment is a type question first (O(1)); converting
    // a non-environment's payload before rejecting it would copy it.
    let e = match env.typ() {
        ENVSXP => match crate::bridge::from_sexp(env) {
            robigo_runtime::value::Value::Env(e) => e,
            _ => crate::errors::error_str("not an environment"),
        },
        OBJSXP if env.gp() & robigo_core::gp::S4 != 0 => {
            let slot = crate::objects::get_s4_data_slot_any(env);
            match crate::bridge::from_sexp(slot) {
                robigo_runtime::value::Value::Env(e) => e,
                _ => crate::errors::error_str("not an environment"),
            }
        }
        _ => crate::errors::error_str("not an environment"),
    };
    e.0.locked.get() as c_int
}

// R: coerce.c:substituteList
unsafe fn substitute_list(mut el: Sexp, rho: Sexp) -> Sexp {
    if el == R_NilValue {
        return el;
    }
    let mut res = R_NilValue;
    let mut p = R_NilValue;
    while el != R_NilValue {
        let mut h = if CAR(el) == R_DotsSymbol {
            let d = if rho == R_NilValue { R_UnboundValue } else { Rf_findVarInFrame(rho, CAR(el)) };
            if d == R_UnboundValue {
                Rf_lcons(R_DotsSymbol, R_NilValue)
            } else if d == R_NilValue || d == R_MissingArg {
                R_NilValue
            } else if d.typ() == DOTSXP {
                Rf_protect(d);
                let sub = substitute_list(d, R_NilValue);
                Rf_unprotect(1);
                sub
            } else {
                crate::errors::error_str("'...' used in an incorrect context")
            }
        } else {
            let sub = Rf_protect(Rf_substitute(CAR(el), rho));
            MARK_NOT_MUTABLE(sub);
            let node =
                if crate::util::Rf_isLanguage(el) != 0 { Rf_lcons(sub, R_NilValue) } else { Rf_cons(sub, R_NilValue) };
            Rf_unprotect(1);
            SET_TAG(node, TAG(el));
            node
        };
        if h != R_NilValue {
            if res == R_NilValue {
                res = Rf_protect(h);
            } else {
                SETCDR(p, h);
            }
            while CDR(h) != R_NilValue {
                h = CDR(h);
            }
            p = h;
        }
        el = CDR(el);
    }
    if res != R_NilValue {
        Rf_unprotect(1);
    }
    res
}

// R: coerce.c:substitute
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_substitute(lang: Sexp, rho: Sexp) -> Sexp {
    match lang.typ() {
        PROMSXP => Rf_substitute(PRCODE(lang), rho),
        SYMSXP => {
            if rho != R_NilValue {
                let mut t = Rf_findVarInFrame(rho, lang);
                if t != R_UnboundValue {
                    if t.typ() == PROMSXP {
                        while t.typ() == PROMSXP {
                            t = PRCODE(t);
                        }
                        MARK_NOT_MUTABLE(t);
                        return t;
                    } else if t.typ() == DOTSXP {
                        crate::errors::error_str("'...' used in an incorrect context")
                    }
                    if rho != R_GlobalEnv {
                        return t;
                    }
                }
            }
            lang
        }
        LANGSXP => substitute_list(lang, rho),
        _ => lang,
    }
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
