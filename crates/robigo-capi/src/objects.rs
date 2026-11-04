//! The S4 entry points of the C API: objects.c (`R_do_MAKE_CLASS`,
//! `R_getClassDef`, `R_do_new_object`, `asS4`) and attrib.c's slots
//! (`R_do_slot`, `R_do_slot_assign`, `R_has_slot`, `R_getS4DataSlot`).
//! They work on core objects in place, as GNU R's do (Rcpp modules keep
//! the object they assign slots to); the runtime's own S4 code is
//! crates/robigo-runtime/src/s4.rs.

use crate::attrib::{Rf_getAttrib, Rf_setAttrib, install_attrib};
use crate::coerce::Rf_asChar;
use crate::envir::{R_FindNamespace, Rf_eval};
use crate::errors::{error_str, error_str_owned};
use crate::list::{Rf_ScalarLogical, Rf_lang2, Rf_lang3, Rf_lang6};
use crate::memory::*;
use crate::state::*;
use crate::strings::{Rf_ScalarString, Rf_installTrChar, Rf_mkChar, Rf_mkString};
use crate::util::{Rf_type2str, type_to_char};
use robigo_core::types::*;
use robigo_core::{Sexp, gp};
use std::ffi::{CStr, c_char, c_int};

unsafe fn sym(name: &str) -> Sexp {
    core().install(name.as_bytes())
}

unsafe fn chars(c: Sexp) -> String {
    CStr::from_ptr(R_CHAR(c)).to_string_lossy().into_owned()
}

/// `CHAR(asChar(x))`.
unsafe fn as_char(x: Sexp) -> String {
    chars(crate::coerce::Rf_asChar(x))
}

/// `R_MethodsNamespace`.
unsafe fn methods_namespace() -> Sexp {
    R_FindNamespace(Rf_mkString(c"methods".as_ptr()))
}

/// The first element of `R_data_class(obj, FALSE)`.
unsafe fn data_class1(obj: Sexp) -> String {
    chars(STRING_ELT(R_data_class(obj, 0), 0))
}

/// The class of a call: its function's name for the syntactic calls,
/// otherwise `"call"`.
// R: attrib.c:lang2str
unsafe fn lang2str(obj: Sexp) -> Sexp {
    let symb = CAR(obj);
    if symb.typ() == SYMSXP {
        for name in ["if", "for", "while", "(", "{", "=", "<-"] {
            if symb == sym(name) {
                return PRINTNAME(symb);
            }
        }
    }
    PRINTNAME(sym("call"))
}

// R: attrib.c:R_data_class
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_data_class(obj: Sexp, single_string: c_int) -> Sexp {
    let mut klass = Rf_getAttrib(obj, R_ClassSymbol);
    let n = Rf_length(klass);
    if n == 1 || (n > 0 && single_string == 0) {
        return klass;
    }
    if n == 0 {
        let dim = Rf_getAttrib(obj, R_DimSymbol);
        let nd = Rf_length(dim);
        if nd > 0 {
            if nd == 2 {
                if single_string != 0 {
                    klass = Rf_mkChar(c"matrix".as_ptr());
                } else {
                    // R >= 4.0.0: class(<matrix>) |-> c("matrix", "array")
                    klass = Rf_protect(Rf_allocVector(STRSXP as _, 2));
                    SET_STRING_ELT(klass, 0, Rf_mkChar(c"matrix".as_ptr()));
                    SET_STRING_ELT(klass, 1, Rf_mkChar(c"array".as_ptr()));
                    Rf_unprotect(1);
                    return klass;
                }
            } else {
                klass = Rf_mkChar(c"array".as_ptr());
            }
        } else {
            klass = match obj.typ() {
                CLOSXP | SPECIALSXP | BUILTINSXP => Rf_mkChar(c"function".as_ptr()),
                REALSXP => Rf_mkChar(c"numeric".as_ptr()),
                SYMSXP => Rf_mkChar(c"name".as_ptr()),
                LANGSXP => lang2str(obj),
                OBJSXP => Rf_mkChar(if IS_S4_OBJECT(obj) != 0 { c"S4" } else { c"object" }.as_ptr()),
                t => Rf_type2str(t as _),
            };
        }
    } else {
        klass = Rf_asChar(klass);
    }
    Rf_protect(klass);
    let value = Rf_ScalarString(klass);
    Rf_unprotect(1);
    value
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
unsafe fn check_class_and_super(x: Sexp, valid: *const *const c_char, mut rho: Sexp) -> c_int {
    if OBJECT(x) == 0 {
        return -1;
    }
    let clattr = Rf_getAttrib(x, R_ClassSymbol);
    let cl = Rf_protect(Rf_asChar(clattr));
    let class = CStr::from_ptr(R_CHAR(cl));
    if let Some(ans) = valid_index(class, valid) {
        Rf_unprotect(1); // cl
        return ans;
    }
    // if not found directly, then look for a match among the nonvirtual
    // superclasses, possibly after finding the environment 'rho' in which
    // class(x) is defined
    if IS_S4_OBJECT(x) != 0 {
        if rho.is_null() {
            let pkg = Rf_getAttrib(clattr, R_PackageSymbol);
            if pkg != R_NilValue {
                let cl_env_call = Rf_protect(Rf_lang2(sym(".classEnv"), clattr));
                rho = Rf_eval(cl_env_call, methods_namespace());
                Rf_unprotect(1); // cl_env_call
                if rho.typ() != ENVSXP {
                    error_str("could not find correct environment; please report!")
                }
            } else {
                rho = R_GlobalEnv;
            }
        }
        Rf_protect(rho);
        let class_def = Rf_protect(R_getClassDef(R_CHAR(cl)));
        let class_exts = Rf_protect(R_do_slot(class_def, sym("contains")));
        // .selectSuperClasses(getClassDef(class)@contains,
        //                     dropVirtual = TRUE, namesOnly = TRUE,
        //                     directOnly = FALSE, simpleOnly = TRUE):
        let call = Rf_protect(Rf_lang6(
            sym(".selectSuperClasses"),
            class_exts,
            Rf_ScalarLogical(1),
            Rf_ScalarLogical(1),
            Rf_ScalarLogical(0),
            Rf_ScalarLogical(1),
        ));
        let super_cl = Rf_eval(call, rho);
        Rf_unprotect(3); // call, class_exts, class_def
        Rf_protect(super_cl);
        for i in 0..LENGTH(super_cl) {
            let s_class = CStr::from_ptr(R_CHAR(STRING_ELT(super_cl, i as isize)));
            if let Some(ans) = valid_index(s_class, valid) {
                Rf_unprotect(3); // super_cl, rho, cl
                return ans;
            }
        }
        Rf_unprotect(2); // super_cl, rho
    }
    Rf_unprotect(1); // cl
    -1
}

// R: objects.c:R_check_class_etc
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_check_class_etc(x: Sexp, valid: *const *const c_char) -> c_int {
    check_class_and_super(x, valid, Sexp::NULL)
}

// R: objects.c:R_do_MAKE_CLASS
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_do_MAKE_CLASS(what: *const c_char) -> Sexp {
    if what.is_null() {
        error_str("C level MAKE_CLASS macro called with NULL string pointer")
    }
    let call = Rf_protect(Rf_lang2(sym("getClass"), Rf_mkString(what)));
    let e = Rf_eval(call, methods_namespace());
    Rf_unprotect(1);
    e
}

// R: objects.c:R_getClassDef
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_getClassDef(what: *const c_char) -> Sexp {
    if what.is_null() {
        error_str("R_getClassDef(.) called with NULL string pointer")
    }
    let call = Rf_protect(Rf_lang2(sym("getClassDef"), Rf_mkString(what)));
    let e = Rf_eval(call, methods_namespace());
    Rf_unprotect(1);
    e
}

/// The slot name as a symbol (`R_SLOT_INIT`).
unsafe fn slot_sym(name: Sexp) -> Sexp {
    if name.typ() == SYMSXP {
        name
    } else if name.typ() == STRSXP && LENGTH(name) == 1 {
        Rf_installTrChar(STRING_ELT(name, 0))
    } else {
        error_str("invalid type or length for slot name")
    }
}

// R: attrib.c:R_has_slot
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_has_slot(obj: Sexp, name: Sexp) -> c_int {
    let name = slot_sym(name);
    if name == sym(".Data") && obj.typ() != OBJSXP {
        return 1;
    }
    (Rf_getAttrib(obj, name) != R_NilValue) as c_int
}

// R: attrib.c:R_do_slot
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_do_slot(obj: Sexp, name: Sexp) -> Sexp {
    let name = slot_sym(name);
    if name == sym(".Data") {
        // data_part: methods:::getDataPart(obj, TRUE)
        let e = Rf_protect(Rf_lang3(sym("getDataPart"), obj, Rf_ScalarLogical(1)));
        let val = Rf_eval(e, methods_namespace());
        UNSET_S4_OBJECT(val);
        Rf_unprotect(1);
        return val;
    }
    let value = Rf_getAttrib(obj, name);
    if value == R_NilValue {
        if name == sym(".S3Class") {
            let klass = Rf_getAttrib(obj, R_ClassSymbol);
            if klass != R_NilValue {
                return klass;
            }
            return R_data_class(obj, 0);
        }
        if name == R_NamesSymbol && obj.typ() == VECSXP {
            return value;
        }
        let class_string = Rf_getAttrib(obj, R_ClassSymbol);
        let input = chars(PRINTNAME(name));
        let msg = if class_string == R_NilValue {
            format!("cannot get a slot (\"{}\") from an object of type \"{}\"", input, type_to_char(obj))
        } else {
            format!("no slot of name \"{}\" for this object of class \"{}\"", input, as_char(class_string))
        };
        drop(input);
        error_str_owned(msg)
    }
    if value == sym("\u{1}NULL\u{1}") {
        return R_NilValue;
    }
    value
}

// R: attrib.c:R_do_slot_assign
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_do_slot_assign(obj: Sexp, name: Sexp, value: Sexp) -> Sexp {
    if obj == R_NilValue {
        error_str("attempt to set slot on NULL object")
    }
    Rf_protect(obj);
    Rf_protect(value);
    let name = if name.typ() == STRSXP && LENGTH(name) == 1 {
        Rf_installTrChar(STRING_ELT(name, 0))
    } else if name.typ() == CHARSXP {
        Rf_installTrChar(name)
    } else {
        name
    };
    if name.typ() != SYMSXP {
        error_str("invalid type or length for slot name")
    }
    let obj = if name == sym(".Data") {
        // set_data_part: methods:::setDataPart(obj, value)
        let e = Rf_protect(Rf_lang3(sym("setDataPart"), obj, value));
        let val = Rf_eval(e, methods_namespace());
        SET_S4_OBJECT(val);
        Rf_unprotect(1);
        val
    } else {
        let value = if value == R_NilValue { sym("\u{1}NULL\u{1}") } else { value };
        install_attrib(obj, name, value);
        obj
    };
    Rf_unprotect(2);
    obj
}

// R: objects.c:R_do_new_object
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_do_new_object(class_def: Sexp) -> Sexp {
    if class_def.is_null() {
        error_str("C level NEW macro called with null class definition pointer")
    }
    let e = R_do_slot(class_def, sym("virtual"));
    if crate::coerce::Rf_asLogical(e) != 0 {
        let e = R_do_slot(class_def, sym("className"));
        error_str_owned(format!("trying to generate an object from a virtual class (\"{}\")", as_char(e)))
    }
    let e = Rf_protect(R_do_slot(class_def, sym("className")));
    let value = Rf_protect(Rf_duplicate(R_do_slot(class_def, sym("prototype"))));
    let x_data_type = matches!(value.typ(), ENVSXP | SYMSXP | EXTPTRSXP);
    if (value.typ() == OBJSXP || Rf_getAttrib(e, R_PackageSymbol) != R_NilValue) && !x_data_type {
        Rf_setAttrib(value, R_ClassSymbol, e);
        SET_S4_OBJECT(value);
    }
    Rf_unprotect(2);
    value
}

/// `R_getS4DataSlot(obj, ANYSXP)`.
// R: attrib.c:R_getS4DataSlot
pub(crate) unsafe fn get_s4_data_slot_any(obj: Sexp) -> Sexp {
    let obj = Rf_protect(obj);
    let mut value;
    if obj.typ() != OBJSXP {
        let s3class = Rf_protect(Rf_getAttrib(obj, sym(".S3Class")));
        let o = if MAYBE_REFERENCED(obj) != 0 { Rf_shallow_duplicate(obj) } else { obj };
        Rf_protect(o);
        if s3class != R_NilValue {
            Rf_setAttrib(o, R_ClassSymbol, s3class);
            Rf_setAttrib(o, sym(".S3Class"), R_NilValue);
        } else {
            Rf_setAttrib(o, R_ClassSymbol, R_NilValue);
        }
        UNSET_S4_OBJECT(o);
        Rf_unprotect(2);
        value = o;
    } else {
        value = Rf_getAttrib(obj, sym(".Data"));
    }
    if value == R_NilValue {
        value = Rf_getAttrib(obj, sym(".xData"));
    }
    Rf_unprotect(1);
    value
}

// R: objects.c:asS4
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asS4(s: Sexp, flag: c_int, complete: c_int) -> Sexp {
    let is_s4 = s.gp() & gp::S4 != 0;
    if (flag != 0) == is_s4 {
        return s;
    }
    let s = if MAYBE_SHARED(s) != 0 { Rf_shallow_duplicate(s) } else { s };
    Rf_protect(s);
    if flag != 0 {
        SET_S4_OBJECT(s);
    } else {
        if complete != 0 {
            let value = get_s4_data_slot_any(s);
            if value != R_NilValue && value.gp() & gp::S4 == 0 {
                Rf_unprotect(1);
                return value;
            } else if complete == 1 {
                error_str_owned(format!(
                    "object of class \"{}\" does not correspond to a valid S3 object",
                    data_class1(s)
                ))
            } else {
                Rf_unprotect(1);
                return s;
            }
        }
        UNSET_S4_OBJECT(s);
    }
    Rf_unprotect(1);
    s
}

// ---- closures (R 4.5's public accessors) -------------------------------------

/// The runtime closure behind `x` (closures cross the bridge as handles).
unsafe fn closure_of(x: Sexp, what: &str) -> std::rc::Rc<robigo_runtime::value::Closure> {
    let v = crate::bridge::from_sexp(x);
    if let robigo_runtime::value::Value::Closure(c) = v {
        return c;
    }
    drop(v);
    error_str_owned(format!("robigo: {} applied to an object of type '{}', not a closure", what, type_to_char(x)))
}

// R: memory.c:R_ClosureFormals
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ClosureFormals(x: Sexp) -> Sexp {
    let c = closure_of(x, "R_ClosureFormals");
    crate::bridge::to_sexp(&robigo_runtime::value::Value::pairlist(c.formals.to_vec()))
}

// R: memory.c:R_ClosureBody
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ClosureBody(x: Sexp) -> Sexp {
    let c = closure_of(x, "R_ClosureBody");
    crate::bridge::to_sexp(&c.body)
}

// R: eval.c:R_ClosureExpr (the body without its bytecode; robigo's
// closure bodies are always the expression)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ClosureExpr(x: Sexp) -> Sexp {
    let c = closure_of(x, "R_ClosureExpr");
    crate::bridge::to_sexp(&c.body)
}

// R: memory.c:R_ClosureEnv
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ClosureEnv(x: Sexp) -> Sexp {
    let c = closure_of(x, "R_ClosureEnv");
    crate::bridge::to_sexp(&robigo_runtime::value::Value::Env(c.env.clone()))
}

// R: dstruct.c:mkCLOSXP, eval.c:CheckFormals, R_mkClosure
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_mkClosure(formals: Sexp, body: Sexp, rho: Sexp) -> Sexp {
    let mut s = formals;
    if s != R_NilValue && s.typ() != LISTSXP {
        error_str("invalid formal argument list for \"R_mkClosure\"");
    }
    while s != R_NilValue {
        if s.tag().typ() != SYMSXP {
            error_str("invalid formal argument list for \"R_mkClosure\"");
        }
        s = s.cdr();
    }
    let rho_value = crate::bridge::from_sexp(rho);
    let robigo_runtime::value::Value::Env(env) = rho_value else {
        drop(rho_value);
        error_str("invalid environment")
    };
    if matches!(body.typ(), CLOSXP | BUILTINSXP | SPECIALSXP | DOTSXP | ANYSXP) {
        drop(env);
        error_str("invalid body argument for 'function'");
    }
    let formals_vec = match crate::bridge::from_sexp(formals) {
        robigo_runtime::value::Value::Pairlist(p) => p.items.clone(),
        _ => Vec::new(),
    };
    #[cfg(feature = "bytecode-fidelity")]
    let (body_val, gnu_bytecode, jit) = match crate::bridge::from_sexp(body) {
        robigo_runtime::value::Value::Bytecode(b) => {
            (b.expression.clone(), Some(b), robigo_runtime::vm::JitState::from_bytecode())
        }
        v => (v, None, Default::default()),
    };
    #[cfg(not(feature = "bytecode-fidelity"))]
    let (body_val, jit) = (crate::bridge::from_sexp(body), Default::default());
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
    crate::memory::PRCODE(p)
}
