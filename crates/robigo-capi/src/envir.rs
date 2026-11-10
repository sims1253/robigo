//! envir.c, eval.c, context.c (R_tryEval), parse.c and print.c entry
//! points. Until G3 environments live in the host runtime, so these are
//! host operations (`docs/core.md` §6).

use crate::errors::{Res, flow, ret};
use crate::host::{op, try_op};
use crate::memory::*;
use crate::sort::internal;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Sexp, flags};
use std::ffi::{CStr, c_char, c_int};

unsafe fn lgl(b: bool) -> Sexp {
    crate::list::Rf_ScalarLogical(b as c_int)
}

unsafe fn int(x: c_int) -> Sexp {
    crate::list::Rf_ScalarInteger(x)
}

// R: eval.c:eval
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_eval(e: Sexp, rho: Sexp) -> Sexp {
    ret(eval(e, rho))
}

/// eval: the host evaluates.
#[inline]
pub unsafe fn eval(e: Sexp, rho: Sexp) -> Res<Sexp> {
    let mut out = Sexp::NULL;
    flow((crate::host::host().eval)(e, rho, &mut out))?;
    Ok(out)
}

/// R_tryEval / R_tryEvalSilent: evaluate at top level (outer handlers hidden,
/// errors reported by the default handler unless `silent`), never jump.
unsafe fn try_eval(e: Sexp, env: Sexp, err: *mut c_int, silent: bool) -> Sexp {
    let mut tmp = Sexp::NULL;
    try_op("toplevelBegin\0", &[], &mut tmp);
    // a context, as GNU R's R_ToplevelExec: a failure inside the host
    // callback (converting its result) is undone before it is reported
    let r = crate::errors::in_context(|| eval(e, env));
    try_op("toplevelEnd\0", &[], &mut tmp);
    let out = match r {
        Ok(out) => out,
        Err(u) => {
            let t = u.into_token();
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
    };
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

// R: eval.c:R_forceAndCall — a closure's first `n` promised arguments
// are forced before the call (purrr's map forces `.f`'s `.x[[i]]` while
// `i` is the current index)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_forceAndCall(e: Sexp, n: c_int, rho: Sexp) -> Sexp {
    ret(force_and_call(e, n, rho))
}

unsafe fn force_and_call(e: Sexp, n: c_int, rho: Sexp) -> Res<Sexp> {
    let n = protect(crate::list::Rf_ScalarInteger(n))?;
    let r = op("forceAndCall\0", &[e, n, rho])?;
    unprotect(1)?;
    Ok(r)
}

// R: envir.c:R_NewEnv
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_NewEnv(enclos: Sexp, _hash: c_int, _size: c_int) -> Sexp {
    ret(op("newEnv\0", &[enclos]))
}

// R: envir.c:NewEnvironment — the frame is valuelist, its tags taken
// from namelist where that has elements
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_NewEnvironment(namelist: Sexp, valuelist: Sexp, rho: Sexp) -> Sexp {
    ret(new_environment(namelist, valuelist, rho))
}

unsafe fn new_environment(namelist: Sexp, valuelist: Sexp, rho: Sexp) -> Res<Sexp> {
    let env = protect(op("newEnv\0", &[rho])?)?;
    let (mut v, mut n) = (valuelist, namelist);
    // the values are rooted through valuelist
    let mut frame = Vec::with_capacity(r_length(valuelist)? as usize);
    while v != R_NilValue {
        let tag = if n != R_NilValue { field(n, 2)? } else { field(v, 2)? };
        frame.push((tag, field(v, 0)?));
        v = field(v, 1)?;
        if n != R_NilValue {
            n = field(n, 1)?;
        }
    }
    // defineVar keeps the first binding of a name, as a frame lookup does
    for &(tag, val) in frame.iter().rev() {
        if tag != R_NilValue {
            define_var(tag, val, env)?;
        }
    }
    unprotect(1)?;
    Ok(env)
}

// R: envir.c:defineVar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_defineVar(sym: Sexp, val: Sexp, rho: Sexp) {
    ret(define_var(sym, val, rho))
}

/// defineVar.
pub unsafe fn define_var(sym: Sexp, val: Sexp, rho: Sexp) -> Res<()> {
    op("defineVar\0", &[sym, val, rho])?;
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_setVar(sym: Sexp, val: Sexp, rho: Sexp) {
    ret(op("setVar\0", &[sym, val, rho]));
}

// R: envir.c:findVar — the binding's value (promises unforced), through
// the enclosures; R_UnboundValue if none
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_findVar(sym: Sexp, rho: Sexp) -> Sexp {
    ret(op("findVar\0", &[sym, rho]))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_findVarInFrame(rho: Sexp, sym: Sexp) -> Sexp {
    ret(find_var_in_frame(rho, sym))
}

/// findVarInFrame.
pub unsafe fn find_var_in_frame(rho: Sexp, sym: Sexp) -> Res<Sexp> {
    op("findVarInFrame\0", &[rho, sym])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_findVarInFrame3(rho: Sexp, sym: Sexp, _doget: c_int) -> Sexp {
    ret(find_var_in_frame(rho, sym))
}

/// `*LOGICAL(x) == TRUE` of a host operation's result.
unsafe fn is_true(x: Res<Sexp>) -> Res<c_int> {
    Ok((*logical(x?)? == 1) as c_int)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_existsVarInFrame(rho: Sexp, sym: Sexp) -> c_int {
    ret(is_true(op("existsVarInFrame\0", &[rho, sym])))
}

// R: envir.c:R_removeVarFromFrame (name, env — the other order)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_removeVarFromFrame(name: Sexp, env: Sexp) {
    ret(op("removeVar\0", &[name, env]));
}

// R: envir.c:findFun
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_findFun(sym: Sexp, rho: Sexp) -> Sexp {
    ret(op("findFun\0", &[sym, rho]))
}

// R: envir.c:R_getVar — forces promises; "object 'x' not found"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_getVar(sym: Sexp, rho: Sexp, inherits: c_int) -> Sexp {
    ret(op("getVar\0", &[sym, rho, lgl(inherits != 0), R_UnboundValue]))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_getVarEx(sym: Sexp, rho: Sexp, inherits: c_int, ifnotfound: Sexp) -> Sexp {
    ret(op("getVar\0", &[sym, rho, lgl(inherits != 0), ifnotfound]))
}

// ---- the environment body (council item 20) --------------------------------
// Only ENCLOS has a fixed place in an ENVSXP; FRAME and HASHTAB are shims
// (robigo_core::env) that materialize from the frame words, so G3 can give
// environments slot frames. A HOST environment (the runtime's, before G3)
// asks the host, which builds the pairlist of bindings. GNU R's accessors
// do not check the type: on another node they read the shared field.

// R: memory.c:FRAME
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FRAME(x: Sexp) -> Sexp {
    if x.has_flag(flags::HOST) {
        return ret(field(x, 0));
    }
    if x.typ() == ENVSXP { robigo_core::env::frame(x) } else { x.car() }
}

// R: memory.c:ENCLOS
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ENCLOS(x: Sexp) -> Sexp {
    if x.has_flag(flags::HOST) {
        return ret(field(x, 1));
    }
    if x.typ() == ENVSXP { robigo_core::env::enclos(x) } else { x.cdr() }
}

// R: memory.c:HASHTAB
#[unsafe(no_mangle)]
pub unsafe extern "C" fn HASHTAB(x: Sexp) -> Sexp {
    if x.has_flag(flags::HOST) {
        return ret(field(x, 2));
    }
    if x.typ() == ENVSXP { robigo_core::env::hashtab(x) } else { x.tag() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ParentEnv(env: Sexp) -> Sexp {
    if env.typ() != ENVSXP {
        crate::errors::throw(crate::errors::error_str("argument is not an environment"))
    }
    ret(field(env, 1))
}

// R: envir.c:R_lsInternal3
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_lsInternal3(env: Sexp, all: c_int, sorted: c_int) -> Sexp {
    ret(internal("ls", &[env, lgl(all != 0), lgl(sorted != 0)]))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_lsInternal(env: Sexp, all: c_int) -> Sexp {
    ret(internal("ls", &[env, lgl(all != 0), lgl(true)]))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_MakeActiveBinding(sym: Sexp, fun: Sexp, env: Sexp) {
    ret(internal("makeActiveBinding", &[sym, fun, env]));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_BindingIsActive(sym: Sexp, env: Sexp) -> c_int {
    ret(is_true(internal("bindingIsActive", &[sym, env])))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_BindingIsLocked(sym: Sexp, env: Sexp) -> c_int {
    ret(is_true(internal("bindingIsLocked", &[sym, env])))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ActiveBindingFunction(sym: Sexp, env: Sexp) -> Sexp {
    ret(internal("activeBindingFunction", &[sym, env]))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_LockBinding(sym: Sexp, env: Sexp) {
    ret(internal("lockBinding", &[sym, env]));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_unLockBinding(sym: Sexp, env: Sexp) {
    ret(internal("unlockBinding", &[sym, env]));
}

// R: envir.c:R_LockEnvironment
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_LockEnvironment(env: Sexp, bindings: c_int) {
    ret(internal("lockEnvironment", &[env, lgl(bindings != 0)]));
}

// R: envir.c:R_PackageEnvName
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_PackageEnvName(rho: Sexp) -> Sexp {
    ret(package_env_name(rho))
}

unsafe fn package_env_name(rho: Sexp) -> Res<Sexp> {
    if !rho.is_null() && rho.typ() == ENVSXP {
        let name = crate::attrib::get_attrib(rho, R_NameSymbol)?;
        if name.typ() == STRSXP && length(name)? > 0 && string_elt(name, 0)?.char_bytes().starts_with(b"package:") {
            return Ok(name);
        }
    }
    Ok(R_NilValue)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_IsNamespaceEnv(rho: Sexp) -> c_int {
    ret(is_true(internal("isNamespaceEnv", &[rho])))
}

// R: envir.c:R_IsPackageEnv — an attached package environment, named
// "package:foo". TYPEOF gates before anything else (O(1), as in R;
// `from_sexp` would materialize a non-environment's payload).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_IsPackageEnv(rho: Sexp) -> c_int {
    if rho.typ() != ENVSXP {
        return 0;
    }
    match ret(crate::bridge::from_sexp(rho)) {
        robigo_runtime::value::Value::Env(e) => e.package_env_name().is_some() as c_int,
        _ => 0,
    }
}

// R: envir.c:R_EnvironmentIsLocked (simple_as_environment: an S4
// object's environment data slot counts as the environment)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_EnvironmentIsLocked(env: Sexp) -> c_int {
    ret(environment_is_locked(env))
}

unsafe fn environment_is_locked(env: Sexp) -> Res<c_int> {
    use crate::errors::error_str;
    // R checks NILSXP before anything else.
    if env == R_NilValue || env.is_null() {
        return Err(error_str("use of NULL environment is defunct"));
    }
    // simple_as_environment is a type question first (O(1)); converting
    // a non-environment's payload before rejecting it would copy it.
    let e = match env.typ() {
        ENVSXP => match crate::bridge::from_sexp(env)? {
            robigo_runtime::value::Value::Env(e) => e,
            _ => return Err(error_str("not an environment")),
        },
        OBJSXP if env.gp() & robigo_core::gp::S4 != 0 => {
            let slot = crate::objects::get_s4_data_slot_any(env)?;
            match crate::bridge::from_sexp(slot)? {
                robigo_runtime::value::Value::Env(e) => e,
                _ => return Err(error_str("not an environment")),
            }
        }
        _ => return Err(error_str("not an environment")),
    };
    Ok(e.0.locked.get() as c_int)
}

// R: coerce.c:substituteList
unsafe fn substitute_list(mut el: Sexp, rho: Sexp) -> Res<Sexp> {
    if el == R_NilValue {
        return Ok(el);
    }
    let mut res = R_NilValue;
    let mut p = R_NilValue;
    while el != R_NilValue {
        let car = field(el, 0)?;
        let mut h = if car == R_DotsSymbol {
            let d = if rho == R_NilValue { R_UnboundValue } else { find_var_in_frame(rho, car)? };
            if d == R_UnboundValue {
                Rf_lcons(R_DotsSymbol, R_NilValue)
            } else if d == R_NilValue || d == R_MissingArg {
                R_NilValue
            } else if d.typ() == DOTSXP {
                protect(d)?;
                let sub = substitute_list(d, R_NilValue)?;
                unprotect(1)?;
                sub
            } else {
                return Err(crate::errors::error_str("'...' used in an incorrect context"));
            }
        } else {
            let sub = protect(substitute(car, rho)?)?;
            MARK_NOT_MUTABLE(sub);
            let node =
                if crate::util::Rf_isLanguage(el) != 0 { Rf_lcons(sub, R_NilValue) } else { Rf_cons(sub, R_NilValue) };
            unprotect(1)?;
            set_tag(node, field(el, 2)?)?;
            node
        };
        if h != R_NilValue {
            if res == R_NilValue {
                res = protect(h)?;
            } else {
                setcdr(p, h)?;
            }
            while field(h, 1)? != R_NilValue {
                h = field(h, 1)?;
            }
            p = h;
        }
        el = field(el, 1)?;
    }
    if res != R_NilValue {
        unprotect(1)?;
    }
    Ok(res)
}

// R: coerce.c:substitute
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_substitute(lang: Sexp, rho: Sexp) -> Sexp {
    ret(substitute(lang, rho))
}

unsafe fn substitute(lang: Sexp, rho: Sexp) -> Res<Sexp> {
    match lang.typ() {
        PROMSXP => substitute(field(lang, 1)?, rho),
        SYMSXP => {
            if rho != R_NilValue {
                let mut t = find_var_in_frame(rho, lang)?;
                if t != R_UnboundValue {
                    if t.typ() == PROMSXP {
                        while t.typ() == PROMSXP {
                            t = field(t, 1)?;
                        }
                        MARK_NOT_MUTABLE(t);
                        return Ok(t);
                    } else if t.typ() == DOTSXP {
                        return Err(crate::errors::error_str("'...' used in an incorrect context"));
                    }
                    if rho != R_GlobalEnv {
                        return Ok(t);
                    }
                }
            }
            Ok(lang)
        }
        LANGSXP => substitute_list(lang, rho),
        _ => Ok(lang),
    }
}

// R: envir.c:R_FindNamespace — calls getNamespace(info) in base
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_FindNamespace(info: Sexp) -> Sexp {
    ret(find_namespace(info))
}

unsafe fn find_namespace(info: Sexp) -> Res<Sexp> {
    let f = core().install(b"getNamespace");
    let call = protect(crate::list::lang2(f, info)?)?;
    let r = eval(call, R_GlobalEnv)?;
    unprotect(1)?;
    Ok(r)
}

// R: unique.c:match
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_match(table: Sexp, x: Sexp, nomatch: c_int) -> Sexp {
    ret(internal("match", &[x, table, int(nomatch), R_NilValue]))
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
    ret(op("printValue\0", &[x]));
}

// R: sysutils.c:R_ExpandFileName (a static buffer, as GNU R's)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ExpandFileName(s: *const c_char) -> *const c_char {
    thread_local! {
        static BUF: std::cell::UnsafeCell<[u8; 4096]> = const { std::cell::UnsafeCell::new([0; 4096]) };
    }
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
    let buf = BUF.with(|b| b.get()).cast::<u8>();
    std::ptr::copy_nonoverlapping(b.as_ptr(), buf, n);
    *buf.add(n) = 0;
    buf as *const c_char
}
