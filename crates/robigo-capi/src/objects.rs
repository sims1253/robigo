//! The S4 entry points of the C API: objects.c (`R_do_MAKE_CLASS`,
//! `R_getClassDef`, `R_do_new_object`, `asS4`) and attrib.c's slots
//! (`R_do_slot`, `R_do_slot_assign`, `R_has_slot`, `R_getS4DataSlot`).
//! They work on core objects in place, as GNU R's do (Rcpp modules keep
//! the object they assign slots to); the runtime's own S4 code is
//! crates/robigo-runtime/src/s4.rs.

use crate::attrib::{get_attrib, install_attrib, set_attrib};
use crate::envir::eval;
use crate::errors::{Res, error_str, ret};
use crate::list::{Rf_ScalarLogical, lang2, lang3};
use crate::memory::*;
use crate::state::*;
use crate::strings::{install_tr_char, mk_char, mk_string, scalar_string};
use crate::util::type_to_char;
use robigo_core::types::*;
use robigo_core::{Sexp, gp};
use std::ffi::{CStr, c_char, c_int};

unsafe fn sym(name: &str) -> Sexp {
    core().install(name.as_bytes())
}

unsafe fn chars(c: Sexp) -> Res<String> {
    Ok(CStr::from_ptr(r_char(c)?).to_string_lossy().into_owned())
}

/// `CHAR(asChar(x))`.
unsafe fn as_char(x: Sexp) -> Res<String> {
    chars(crate::coerce::as_char(x)?)
}

/// `R_MethodsNamespace`.
unsafe fn methods_namespace() -> Res<Sexp> {
    let f = core().install(b"getNamespace");
    let call = protect(lang2(f, mk_string(c"methods".as_ptr())?)?)?;
    let r = eval(call, R_GlobalEnv)?;
    unprotect(1)?;
    Ok(r)
}

/// `eval(call, R_MethodsNamespace)`, `call` protected meanwhile.
unsafe fn eval_in_methods(call: Sexp) -> Res<Sexp> {
    protect(call)?;
    let ns = methods_namespace()?;
    let r = eval(call, ns)?;
    unprotect(1)?;
    Ok(r)
}

/// The first element of `R_data_class(obj, FALSE)`.
unsafe fn data_class1(obj: Sexp) -> Res<String> {
    chars(string_elt(data_class(obj, 0)?, 0)?)
}

/// The class of a call: its function's name for the syntactic calls,
/// otherwise `"call"`.
// R: attrib.c:lang2str
unsafe fn lang2str(obj: Sexp) -> Res<Sexp> {
    let symb = field(obj, 0)?;
    if symb.typ() == SYMSXP {
        for name in ["if", "for", "while", "(", "{", "=", "<-"] {
            if symb == sym(name) {
                return field(symb, 0);
            }
        }
    }
    field(sym("call"), 0)
}

// R: attrib.c:R_data_class
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_data_class(obj: Sexp, single_string: c_int) -> Sexp {
    ret(data_class(obj, single_string))
}

/// R_data_class.
pub unsafe fn data_class(obj: Sexp, single_string: c_int) -> Res<Sexp> {
    let mut klass = get_attrib(obj, R_ClassSymbol)?;
    let n = r_length(klass)?;
    if n == 1 || (n > 0 && single_string == 0) {
        return Ok(klass);
    }
    if n == 0 {
        let dim = get_attrib(obj, R_DimSymbol)?;
        let nd = r_length(dim)?;
        if nd > 0 {
            if nd == 2 {
                if single_string != 0 {
                    klass = mk_char(c"matrix".as_ptr())?;
                } else {
                    // R >= 4.0.0: class(<matrix>) |-> c("matrix", "array")
                    klass = protect(alloc_vector(STRSXP as _, 2)?)?;
                    set_string_elt(klass, 0, mk_char(c"matrix".as_ptr())?)?;
                    set_string_elt(klass, 1, mk_char(c"array".as_ptr())?)?;
                    unprotect(1)?;
                    return Ok(klass);
                }
            } else {
                klass = mk_char(c"array".as_ptr())?;
            }
        } else {
            klass = match obj.typ() {
                CLOSXP | SPECIALSXP | BUILTINSXP => mk_char(c"function".as_ptr())?,
                REALSXP => mk_char(c"numeric".as_ptr())?,
                SYMSXP => mk_char(c"name".as_ptr())?,
                LANGSXP => lang2str(obj)?,
                OBJSXP => mk_char(if IS_S4_OBJECT(obj) != 0 { c"S4" } else { c"object" }.as_ptr())?,
                t => crate::util::type2str(t as _)?,
            };
        }
    } else {
        klass = crate::coerce::as_char(klass)?;
    }
    protect(klass)?;
    let value = scalar_string(klass)?;
    unprotect(1)?;
    Ok(value)
}

/// Whether the C string `s` is one of `valid` (a `""`-terminated array):
/// its index, if so.
unsafe fn valid_index(s: &CStr, valid: *const *const c_char) -> Option<c_int> {
    let mut ans = 0;
    loop {
        let v = CStr::from_ptr(*valid.add(ans));
        if v.is_empty() {
            return None;
        }
        if v == s {
            return Some(ans as c_int);
        }
        ans += 1;
    }
}

/// The index of `class(x)` in `valid`, a `""`-terminated array, looking
/// also among the non-virtual superclasses of an S4 class (defined in
/// `rho`, else found by `.classEnv`); -1 if none matches. Rinternals.h
/// declares it, but GNU R's libR does not export it.
// R: objects.c:R_check_class_and_super
unsafe fn check_class_and_super(x: Sexp, valid: *const *const c_char, mut rho: Sexp) -> Res<c_int> {
    if OBJECT(x) == 0 {
        return Ok(-1);
    }
    let clattr = get_attrib(x, R_ClassSymbol)?;
    let cl = protect(crate::coerce::as_char(clattr)?)?;
    let class = CStr::from_ptr(r_char(cl)?);
    if let Some(ans) = valid_index(class, valid) {
        unprotect(1)?; // cl
        return Ok(ans);
    }
    // if not found directly, then look for a match among the nonvirtual
    // superclasses, possibly after finding the environment 'rho' in which
    // class(x) is defined
    if IS_S4_OBJECT(x) != 0 {
        if rho.is_null() {
            let pkg = get_attrib(clattr, R_PackageSymbol)?;
            if pkg != R_NilValue {
                let cl_env_call = lang2(sym(".classEnv"), clattr)?;
                rho = eval_in_methods(cl_env_call)?;
                if rho.typ() != ENVSXP {
                    return Err(error_str("could not find correct environment; please report!"));
                }
            } else {
                rho = R_GlobalEnv;
            }
        }
        protect(rho)?;
        let class_def = protect(get_class_def(r_char(cl)?)?)?;
        let class_exts = protect(do_slot(class_def, sym("contains"))?)?;
        // .selectSuperClasses(getClassDef(class)@contains,
        //                     dropVirtual = TRUE, namesOnly = TRUE,
        //                     directOnly = FALSE, simpleOnly = TRUE):
        let t = Rf_ScalarLogical(1);
        let f = Rf_ScalarLogical(0);
        let call = protect(Rf_lcons(sym(".selectSuperClasses"), crate::list::list5(class_exts, t, t, f, t)?))?;
        let super_cl = eval(call, rho)?;
        unprotect(3)?; // call, class_exts, class_def
        protect(super_cl)?;
        for i in 0..length(super_cl)? {
            let s_class = CStr::from_ptr(r_char(string_elt(super_cl, i as isize)?)?);
            if let Some(ans) = valid_index(s_class, valid) {
                unprotect(3)?; // super_cl, rho, cl
                return Ok(ans);
            }
        }
        unprotect(2)?; // super_cl, rho
    }
    unprotect(1)?; // cl
    Ok(-1)
}

// R: objects.c:R_check_class_etc
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_check_class_etc(x: Sexp, valid: *const *const c_char) -> c_int {
    ret(check_class_and_super(x, valid, Sexp::NULL))
}

// R: objects.c:R_do_MAKE_CLASS
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_do_MAKE_CLASS(what: *const c_char) -> Sexp {
    ret(do_make_class(what))
}

unsafe fn do_make_class(what: *const c_char) -> Res<Sexp> {
    if what.is_null() {
        return Err(error_str("C level MAKE_CLASS macro called with NULL string pointer"));
    }
    eval_in_methods(lang2(sym("getClass"), mk_string(what)?)?)
}

// R: objects.c:R_getClassDef
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_getClassDef(what: *const c_char) -> Sexp {
    ret(get_class_def(what))
}

unsafe fn get_class_def(what: *const c_char) -> Res<Sexp> {
    if what.is_null() {
        return Err(error_str("R_getClassDef(.) called with NULL string pointer"));
    }
    eval_in_methods(lang2(sym("getClassDef"), mk_string(what)?)?)
}

/// The slot name as a symbol (`R_SLOT_INIT`).
unsafe fn slot_sym(name: Sexp) -> Res<Sexp> {
    if name.typ() == SYMSXP {
        Ok(name)
    } else if name.typ() == STRSXP && length(name)? == 1 {
        install_tr_char(string_elt(name, 0)?)
    } else {
        Err(error_str("invalid type or length for slot name"))
    }
}

// R: attrib.c:R_has_slot
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_has_slot(obj: Sexp, name: Sexp) -> c_int {
    ret(has_slot(obj, name))
}

unsafe fn has_slot(obj: Sexp, name: Sexp) -> Res<c_int> {
    let name = slot_sym(name)?;
    if name == sym(".Data") && obj.typ() != OBJSXP {
        return Ok(1);
    }
    Ok((get_attrib(obj, name)? != R_NilValue) as c_int)
}

// R: attrib.c:R_do_slot
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_do_slot(obj: Sexp, name: Sexp) -> Sexp {
    ret(do_slot(obj, name))
}

/// R_do_slot.
pub unsafe fn do_slot(obj: Sexp, name: Sexp) -> Res<Sexp> {
    let name = slot_sym(name)?;
    if name == sym(".Data") {
        // data_part: methods:::getDataPart(obj, TRUE)
        let val = eval_in_methods(lang3(sym("getDataPart"), obj, Rf_ScalarLogical(1))?)?;
        UNSET_S4_OBJECT(val);
        return Ok(val);
    }
    let value = get_attrib(obj, name)?;
    if value == R_NilValue {
        if name == sym(".S3Class") {
            let klass = get_attrib(obj, R_ClassSymbol)?;
            if klass != R_NilValue {
                return Ok(klass);
            }
            return data_class(obj, 0);
        }
        if name == R_NamesSymbol && obj.typ() == VECSXP {
            return Ok(value);
        }
        let class_string = get_attrib(obj, R_ClassSymbol)?;
        let input = chars(name.car())?;
        let msg = if class_string == R_NilValue {
            format!("cannot get a slot (\"{}\") from an object of type \"{}\"", input, type_to_char(obj))
        } else {
            format!("no slot of name \"{}\" for this object of class \"{}\"", input, as_char(class_string)?)
        };
        return Err(error_str(&msg));
    }
    if value == sym("\u{1}NULL\u{1}") {
        return Ok(R_NilValue);
    }
    Ok(value)
}

// R: attrib.c:R_do_slot_assign
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_do_slot_assign(obj: Sexp, name: Sexp, value: Sexp) -> Sexp {
    ret(do_slot_assign(obj, name, value))
}

unsafe fn do_slot_assign(obj: Sexp, name: Sexp, value: Sexp) -> Res<Sexp> {
    if obj == R_NilValue {
        return Err(error_str("attempt to set slot on NULL object"));
    }
    protect(obj)?;
    protect(value)?;
    let name = if name.typ() == STRSXP && length(name)? == 1 {
        install_tr_char(string_elt(name, 0)?)?
    } else if name.typ() == CHARSXP {
        install_tr_char(name)?
    } else {
        name
    };
    if name.typ() != SYMSXP {
        return Err(error_str("invalid type or length for slot name"));
    }
    let obj = if name == sym(".Data") {
        // set_data_part: methods:::setDataPart(obj, value)
        let val = eval_in_methods(lang3(sym("setDataPart"), obj, value)?)?;
        SET_S4_OBJECT(val);
        val
    } else {
        let value = if value == R_NilValue { sym("\u{1}NULL\u{1}") } else { value };
        install_attrib(obj, name, value)?;
        obj
    };
    unprotect(2)?;
    Ok(obj)
}

// R: objects.c:R_do_new_object
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_do_new_object(class_def: Sexp) -> Sexp {
    ret(do_new_object(class_def))
}

unsafe fn do_new_object(class_def: Sexp) -> Res<Sexp> {
    if class_def.is_null() {
        return Err(error_str("C level NEW macro called with null class definition pointer"));
    }
    let e = do_slot(class_def, sym("virtual"))?;
    if crate::coerce::as_logical2(e, false, R_NilValue)? != 0 {
        let e = do_slot(class_def, sym("className"))?;
        return Err(error_str(&format!("trying to generate an object from a virtual class (\"{}\")", as_char(e)?)));
    }
    let e = protect(do_slot(class_def, sym("className"))?)?;
    let value = protect(duplicate(do_slot(class_def, sym("prototype"))?)?)?;
    let x_data_type = matches!(value.typ(), ENVSXP | SYMSXP | EXTPTRSXP);
    if (value.typ() == OBJSXP || get_attrib(e, R_PackageSymbol)? != R_NilValue) && !x_data_type {
        set_attrib(value, R_ClassSymbol, e)?;
        SET_S4_OBJECT(value);
    }
    unprotect(2)?;
    Ok(value)
}

/// `R_getS4DataSlot(obj, ANYSXP)`.
// R: attrib.c:R_getS4DataSlot
pub(crate) unsafe fn get_s4_data_slot_any(obj: Sexp) -> Res<Sexp> {
    let obj = protect(obj)?;
    let mut value;
    if obj.typ() != OBJSXP {
        let s3class = protect(get_attrib(obj, sym(".S3Class"))?)?;
        let o = if MAYBE_REFERENCED(obj) != 0 { shallow_duplicate(obj)? } else { obj };
        protect(o)?;
        if s3class != R_NilValue {
            set_attrib(o, R_ClassSymbol, s3class)?;
            set_attrib(o, sym(".S3Class"), R_NilValue)?;
        } else {
            set_attrib(o, R_ClassSymbol, R_NilValue)?;
        }
        UNSET_S4_OBJECT(o);
        unprotect(2)?;
        value = o;
    } else {
        value = get_attrib(obj, sym(".Data"))?;
    }
    if value == R_NilValue {
        value = get_attrib(obj, sym(".xData"))?;
    }
    unprotect(1)?;
    Ok(value)
}

// R: objects.c:asS4
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asS4(s: Sexp, flag: c_int, complete: c_int) -> Sexp {
    ret(as_s4(s, flag, complete))
}

unsafe fn as_s4(s: Sexp, flag: c_int, complete: c_int) -> Res<Sexp> {
    let is_s4 = s.gp() & gp::S4 != 0;
    if (flag != 0) == is_s4 {
        return Ok(s);
    }
    let s = if MAYBE_SHARED(s) != 0 { shallow_duplicate(s)? } else { s };
    protect(s)?;
    if flag != 0 {
        SET_S4_OBJECT(s);
    } else {
        if complete != 0 {
            let value = get_s4_data_slot_any(s)?;
            if value != R_NilValue && value.gp() & gp::S4 == 0 {
                unprotect(1)?;
                return Ok(value);
            } else if complete == 1 {
                return Err(error_str(&format!(
                    "object of class \"{}\" does not correspond to a valid S3 object",
                    data_class1(s)?
                )));
            } else {
                unprotect(1)?;
                return Ok(s);
            }
        }
        UNSET_S4_OBJECT(s);
    }
    unprotect(1)?;
    Ok(s)
}

// ---- closures (R 4.5's public accessors) -------------------------------------

/// The runtime closure behind `x` (closures cross the bridge as handles).
unsafe fn closure_of(x: Sexp, what: &str) -> Res<std::rc::Rc<robigo_runtime::value::Closure>> {
    if let robigo_runtime::value::Value::Closure(c) = crate::bridge::from_sexp(x)? {
        return Ok(c);
    }
    Err(error_str(&format!("robigo: {} applied to an object of type '{}', not a closure", what, type_to_char(x))))
}

// R: memory.c:R_ClosureFormals
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ClosureFormals(x: Sexp) -> Sexp {
    ret(closure_of(x, "R_ClosureFormals")
        .and_then(|c| crate::bridge::to_sexp(&robigo_runtime::value::Value::pairlist(c.formals.to_vec()))))
}

// R: memory.c:R_ClosureBody
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ClosureBody(x: Sexp) -> Sexp {
    ret(closure_of(x, "R_ClosureBody").and_then(|c| crate::bridge::to_sexp(&c.body)))
}

// R: eval.c:R_ClosureExpr (the body without its bytecode; robigo's
// closure bodies are always the expression)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ClosureExpr(x: Sexp) -> Sexp {
    ret(closure_of(x, "R_ClosureExpr").and_then(|c| crate::bridge::to_sexp(&c.body)))
}

// R: memory.c:R_ClosureEnv
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ClosureEnv(x: Sexp) -> Sexp {
    ret(closure_of(x, "R_ClosureEnv")
        .and_then(|c| crate::bridge::to_sexp(&robigo_runtime::value::Value::Env(c.env.clone()))))
}

// R: dstruct.c:mkCLOSXP, eval.c:CheckFormals, R_mkClosure
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_mkClosure(formals: Sexp, body: Sexp, rho: Sexp) -> Sexp {
    ret(mk_closure(formals, body, rho))
}

unsafe fn mk_closure(formals: Sexp, body: Sexp, rho: Sexp) -> Res<Sexp> {
    let mut s = formals;
    if s != R_NilValue && s.typ() != LISTSXP {
        return Err(error_str("invalid formal argument list for \"R_mkClosure\""));
    }
    while s != R_NilValue {
        if s.tag().typ() != SYMSXP {
            return Err(error_str("invalid formal argument list for \"R_mkClosure\""));
        }
        s = s.cdr();
    }
    let robigo_runtime::value::Value::Env(env) = crate::bridge::from_sexp(rho)? else {
        return Err(error_str("invalid environment"));
    };
    if matches!(body.typ(), CLOSXP | BUILTINSXP | SPECIALSXP | DOTSXP | ANYSXP) {
        return Err(error_str("invalid body argument for 'function'"));
    }
    let formals_vec = match crate::bridge::from_sexp(formals)? {
        robigo_runtime::value::Value::Pairlist(p) => p.items.clone(),
        _ => Vec::new(),
    };
    #[cfg_attr(not(feature = "bytecode-fidelity"), allow(unused_variables))]
    let (body_val, gnu_bytecode, jit) = robigo_runtime::value::closure_body(crate::bridge::from_sexp(body)?);
    let clo = robigo_runtime::value::Closure {
        #[cfg(feature = "bytecode-fidelity")]
        gnu_bytecode,
        formals: formals_vec.into(),
        body: body_val,
        env,
        attrs: None,
        code: Default::default(),
        inline: Default::default(),
        defaults: Default::default(),
        jit,
    };
    crate::bridge::to_sexp(&robigo_runtime::value::Value::Closure(std::rc::Rc::new(clo)))
}

// R: eval.c:R_PromiseExpr (the promise expression without its bytecode;
// robigo's promise expressions are always the expression)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_PromiseExpr(p: Sexp) -> Sexp {
    ret(field(p, 1))
}
