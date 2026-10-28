//! The S4 entry points of the C API: objects.c (`R_do_MAKE_CLASS`,
//! `R_getClassDef`, `R_do_new_object`, `asS4`) and attrib.c's slots
//! (`R_do_slot`, `R_do_slot_assign`, `R_has_slot`, `R_getS4DataSlot`).
//! They work on core objects in place, as GNU R's do (Rcpp modules keep
//! the object they assign slots to); the runtime's own S4 code is
//! crates/robigo-runtime/src/s4.rs.

use crate::attrib::{Rf_getAttrib, Rf_setAttrib, install_attrib};
use crate::envir::{R_FindNamespace, Rf_eval};
use crate::errors::{error_str, error_str_owned};
use crate::list::{Rf_ScalarLogical, Rf_lang2, Rf_lang3};
use crate::memory::*;
use crate::state::*;
use crate::strings::{Rf_installTrChar, Rf_mkString};
use crate::util::type_to_char;
use robigo_core::types::*;
use robigo_core::{Sexp, gp};
use std::ffi::{CStr, c_char, c_int};

fn sym(name: &str) -> Sexp {
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
// R: attrib.c:R_data_class
unsafe fn data_class1(obj: Sexp) -> String {
    let klass = Rf_getAttrib(obj, R_ClassSymbol);
    if klass != R_NilValue && LENGTH(klass) > 0 {
        return chars(STRING_ELT(klass, 0));
    }
    let dim = Rf_getAttrib(obj, R_DimSymbol);
    let nd = if dim == R_NilValue { 0 } else { LENGTH(dim) };
    if nd > 0 {
        return if nd == 2 { "matrix".into() } else { "array".into() };
    }
    match obj.typ() {
        CLOSXP | SPECIALSXP | BUILTINSXP => "function".into(),
        REALSXP => "numeric".into(),
        SYMSXP => "name".into(),
        LANGSXP => "call".into(),
        _ => type_to_char(obj).into(),
    }
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
            return Rf_mkString(std::ffi::CString::new(data_class1(obj)).unwrap().as_ptr());
        }
        if name == R_NamesSymbol && obj.typ() == VECSXP {
            return value;
        }
        let input = chars(PRINTNAME(name));
        let class_string = Rf_getAttrib(obj, R_ClassSymbol);
        if class_string == R_NilValue {
            error_str_owned(format!(
                "cannot get a slot (\"{}\") from an object of type \"{}\"",
                input,
                type_to_char(obj)
            ))
        }
        error_str_owned(format!("no slot of name \"{}\" for this object of class \"{}\"", input, as_char(class_string)))
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
unsafe fn get_s4_data_slot_any(obj: Sexp) -> Sexp {
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
