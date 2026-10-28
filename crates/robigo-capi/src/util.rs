//! util.c and Rinlinedfuns.h: type names, predicates, dimensions; and the
//! dispatch points to ALTREP classes and finalizers.

use crate::errors::{error_str, error_str_owned, not_implemented};
use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Rcomplex, Sexp, flags, gp};
use std::ffi::{CStr, c_char, c_int, c_uint};

// R: util.c TypeTable / Type2Table
static TYPE_NAMES: &[(u8, &str)] = &[
    (NILSXP, "NULL"),
    (SYMSXP, "symbol"),
    (LISTSXP, "pairlist"),
    (CLOSXP, "closure"),
    (ENVSXP, "environment"),
    (PROMSXP, "promise"),
    (LANGSXP, "language"),
    (SPECIALSXP, "special"),
    (BUILTINSXP, "builtin"),
    (CHARSXP, "char"),
    (LGLSXP, "logical"),
    (INTSXP, "integer"),
    (REALSXP, "double"),
    (CPLXSXP, "complex"),
    (STRSXP, "character"),
    (DOTSXP, "..."),
    (ANYSXP, "any"),
    (VECSXP, "list"),
    (EXPRSXP, "expression"),
    (BCODESXP, "bytecode"),
    (EXTPTRSXP, "externalptr"),
    (WEAKREFSXP, "weakref"),
    (RAWSXP, "raw"),
    (OBJSXP, "object"),
];

pub fn type_name(t: u8) -> &'static str {
    TYPE_NAMES.iter().find(|(k, _)| *k == t).map(|(_, n)| *n).unwrap_or("unknown")
}

/// R_typeToChar: type2char, but S4 objects are "S4".
pub unsafe fn type_to_char(x: Sexp) -> &'static str {
    if x.typ() == OBJSXP { if x.gp() & gp::S4 != 0 { "S4" } else { "object" } } else { type_name(x.typ()) }
}

static TYPE_CNAMES: &[&CStr] = &[
    c"NULL",
    c"symbol",
    c"pairlist",
    c"closure",
    c"environment",
    c"promise",
    c"language",
    c"special",
    c"builtin",
    c"char",
    c"logical",
    c"",
    c"",
    c"integer",
    c"double",
    c"complex",
    c"character",
    c"...",
    c"any",
    c"list",
    c"expression",
    c"bytecode",
    c"externalptr",
    c"weakref",
    c"raw",
    c"object",
];

// R: util.c:type2char
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_type2char(t: c_uint) -> *const c_char {
    match TYPE_CNAMES.get(t as usize) {
        Some(s) if !s.is_empty() => s.as_ptr(),
        _ => {
            crate::errors::Rf_warning(c"type %d is unimplemented in '%s'".as_ptr(), t as c_int, c"type2char".as_ptr());
            static mut BUF: [u8; 50] = [0; 50];
            let s = format!("unknown type #{t}\0");
            let n = s.len().min(49);
            BUF[..n].copy_from_slice(&s.as_bytes()[..n]);
            std::ptr::addr_of!(BUF) as *const c_char
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_typeToChar(x: Sexp) -> *const c_char {
    if x.typ() == OBJSXP {
        if x.gp() & gp::S4 != 0 { c"S4".as_ptr() } else { c"object".as_ptr() }
    } else {
        Rf_type2char(x.typ() as c_uint)
    }
}

// R: util.c:type2str (a CHARSXP)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_type2str(t: c_uint) -> Sexp {
    match TYPE_CNAMES.get(t as usize) {
        Some(s) if !s.is_empty() => core().mk_char(s.to_bytes(), 0),
        _ => {
            crate::errors::Rf_warning(c"type %d is unimplemented in '%s'".as_ptr(), t as c_int, c"type2str".as_ptr());
            core().mk_char(format!("unknown type #{t}").as_bytes(), 0)
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_type2str_nowarn(t: c_uint) -> Sexp {
    match TYPE_CNAMES.get(t as usize) {
        Some(s) if !s.is_empty() => core().mk_char(s.to_bytes(), 0),
        _ => R_NilValue,
    }
}

// R: util.c:str2type (the TypeTable, with its aliases)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_str2type(s: *const c_char) -> c_uint {
    let s = CStr::from_ptr(s).to_bytes();
    let t: i32 = match s {
        b"NULL" => 0,
        b"symbol" => 1,
        b"pairlist" => 2,
        b"closure" => 3,
        b"environment" => 4,
        b"promise" => 5,
        b"language" => 6,
        b"special" => 7,
        b"builtin" => 8,
        b"char" => 9,
        b"logical" => 10,
        b"integer" => 13,
        b"double" => 14,
        b"complex" => 15,
        b"character" => 16,
        b"..." => 17,
        b"any" => 18,
        b"expression" => 20,
        b"list" => 19,
        b"externalptr" => 22,
        b"bytecode" => 21,
        b"weakref" => 23,
        b"raw" => 24,
        b"S4" | b"object" => 25,
        b"numeric" => 14,
        b"name" => 1,
        _ => -1,
    };
    t as c_uint
}

// ---- predicates (Rinlinedfuns.h, util.c) ------------------------------------

pub unsafe fn is_vector(x: Sexp) -> bool {
    matches!(x.typ(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP | VECSXP | EXPRSXP)
}

unsafe fn inherits(x: Sexp, name: &[u8]) -> bool {
    if !x.has_flag(flags::OBJECT) {
        return false;
    }
    let klass = crate::attrib::get_class(x);
    if klass.typ() != STRSXP {
        return false;
    }
    let n = xlength(klass);
    (0..n).any(|i| STRING_ELT(klass, i).char_bytes() == name)
}

macro_rules! pred {
    ($($name:ident(|$x:ident| $e:expr);)*) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name($x: Sexp) -> c_int {
                ($e) as c_int
            }
        )*
    };
}

pred! {
    Rf_isNull(|s| s.typ() == NILSXP);
    Rf_isSymbol(|s| s.typ() == SYMSXP);
    Rf_isLogical(|s| s.typ() == LGLSXP);
    Rf_isReal(|s| s.typ() == REALSXP);
    Rf_isComplex(|s| s.typ() == CPLXSXP);
    Rf_isExpression(|s| s.typ() == EXPRSXP);
    Rf_isEnvironment(|s| s.typ() == ENVSXP);
    Rf_isString(|s| s.typ() == STRSXP);
    Rf_isObject(|s| s.has_flag(flags::OBJECT));
    Rf_isRaw(|s| s.typ() == RAWSXP);
    Rf_isS4(|s| s.gp() & gp::S4 != 0);
    Rf_isNewList(|s| s.typ() == NILSXP || s.typ() == VECSXP);
    Rf_isList(|s| s.typ() == NILSXP || s.typ() == LISTSXP);
    Rf_isPairList(|s| matches!(s.typ(), NILSXP | LISTSXP | LANGSXP | DOTSXP));
    Rf_isVectorList(|s| matches!(s.typ(), VECSXP | EXPRSXP));
    Rf_isVectorAtomic(|s| matches!(s.typ(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP));
    Rf_isVector(|s| is_vector(s));
    Rf_isPrimitive(|s| matches!(s.typ(), BUILTINSXP | SPECIALSXP));
    Rf_isFunction(|s| matches!(s.typ(), CLOSXP | BUILTINSXP | SPECIALSXP));
    Rf_isLanguage(|s| s.typ() == NILSXP || s.typ() == LANGSXP);
    Rf_isInteger(|s| s.typ() == INTSXP && !inherits(s, b"factor"));
    Rf_isFactor(|s| s.typ() == INTSXP && inherits(s, b"factor"));
    Rf_isNumeric(|s| match s.typ() {
        INTSXP => !inherits(s, b"factor"),
        LGLSXP | REALSXP => true,
        _ => false,
    });
    Rf_isNumber(|s| match s.typ() {
        INTSXP => !inherits(s, b"factor"),
        LGLSXP | REALSXP | CPLXSXP => true,
        _ => false,
    });
    Rf_isFrame(|s| inherits(s, b"data.frame"));
    Rf_isDataFrame(|s| inherits(s, b"data.frame"));
    Rf_isMatrix(|s| is_vector(s) && {
        let t = crate::attrib::Rf_getAttrib(s, R_DimSymbol);
        t.typ() == INTSXP && LENGTH(t) == 2
    });
    Rf_isArray(|s| is_vector(s) && {
        let t = crate::attrib::Rf_getAttrib(s, R_DimSymbol);
        t.typ() == INTSXP && LENGTH(t) > 0
    });
    Rf_isTs(|s| is_vector(s) && crate::attrib::Rf_getAttrib(s, R_TspSymbol) != R_NilValue);
    Rf_isValidString(|s| s.typ() == STRSXP && LENGTH(s) > 0 && STRING_ELT(s, 0).typ() != NILSXP);
    Rf_isUnordered(|s| s.typ() == INTSXP && inherits(s, b"factor") && !inherits(s, b"ordered"));
    Rf_isOrdered(|s| s.typ() == INTSXP && inherits(s, b"factor") && inherits(s, b"ordered"));
    Rf_isValidStringF(|s| Rf_isValidString(s) != 0 && STRING_ELT(s, 0).char_bytes().first().is_some());
}

// R: util.c:isBlankString
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_isBlankString(s: *const c_char) -> c_int {
    CStr::from_ptr(s).to_bytes().iter().all(|c| c.is_ascii_whitespace()) as c_int
}

// R: util.c:StringBlank
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_StringBlank(x: Sexp) -> c_int {
    (x == R_NilValue || x.char_bytes().is_empty()) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_StringFalse(name: *const c_char) -> c_int {
    matches!(CStr::from_ptr(name).to_bytes(), b"F" | b"False" | b"FALSE" | b"false") as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_StringTrue(name: *const c_char) -> c_int {
    matches!(CStr::from_ptr(name).to_bytes(), b"T" | b"True" | b"TRUE" | b"true") as c_int
}

// R: util.c:inherits (via Rinlinedfuns)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_inherits(x: Sexp, name: *const c_char) -> c_int {
    inherits(x, CStr::from_ptr(name).to_bytes()) as c_int
}

// R: util.c:nrows
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_nrows(s: Sexp) -> c_int {
    if is_vector(s) || Rf_isList(s) != 0 {
        let t = crate::attrib::Rf_getAttrib(s, R_DimSymbol);
        if t == R_NilValue {
            return LENGTH(s);
        }
        return *INTEGER(t);
    } else if Rf_isFrame(s) != 0 {
        return Rf_nrows(s.car());
    }
    error_str("object is not a matrix")
}

// R: util.c:ncols
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ncols(s: Sexp) -> c_int {
    if is_vector(s) || Rf_isList(s) != 0 {
        let t = crate::attrib::Rf_getAttrib(s, R_DimSymbol);
        if t == R_NilValue {
            return 1;
        }
        if LENGTH(t) >= 2 {
            return *INTEGER(t).add(1);
        }
        return 1;
    } else if Rf_isFrame(s) != 0 {
        return Rf_length(s);
    }
    error_str("object is not a matrix")
}

// R: util.c:conformable
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_conformable(x: Sexp, y: Sexp) -> c_int {
    let dx = crate::attrib::Rf_getAttrib(x, R_DimSymbol);
    let dy = crate::attrib::Rf_getAttrib(y, R_DimSymbol);
    let n = Rf_length(dx);
    if n != Rf_length(dy) {
        return 0;
    }
    for i in 0..n as usize {
        if *INTEGER(dx).add(i) != *INTEGER(dy).add(i) {
            return 0;
        }
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_envlength(rho: Sexp) -> c_int {
    Rf_envxlength(rho) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_envxlength(rho: Sexp) -> isize {
    let r = crate::host::op("envlength\0", &[rho]);
    *(r.std_data() as *const i32) as isize
}

// ---- ALTREP dispatch (altrep.c) ----------------------------------------------

pub unsafe fn altrep_length(_x: Sexp) -> isize {
    not_implemented("ALTREP_LENGTH")
}
pub unsafe fn altrep_dataptr(_x: Sexp, _writable: bool) -> *mut u8 {
    not_implemented("ALTVEC_DATAPTR")
}
pub unsafe fn altrep_dataptr_or_null(_x: Sexp) -> *mut u8 {
    std::ptr::null_mut()
}
pub unsafe fn altrep_duplicate(_x: Sexp, _deep: bool) -> Sexp {
    not_implemented("ALTREP duplicate")
}
pub unsafe fn altstring_elt(_x: Sexp, _i: isize) -> Sexp {
    not_implemented("ALTSTRING_ELT")
}
pub unsafe fn altstring_set_elt(_x: Sexp, _i: isize, _v: Sexp) {
    not_implemented("ALTSTRING_SET_ELT")
}
pub unsafe fn altlist_elt(_x: Sexp, _i: isize) -> Sexp {
    not_implemented("ALTLIST_ELT")
}
pub unsafe fn altlist_set_elt(_x: Sexp, _i: isize, _v: Sexp) {
    not_implemented("ALTLIST_SET_ELT")
}
pub unsafe fn altinteger_elt(_x: Sexp, _i: isize) -> c_int {
    not_implemented("ALTINTEGER_ELT")
}
pub unsafe fn altlogical_elt(_x: Sexp, _i: isize) -> c_int {
    not_implemented("ALTLOGICAL_ELT")
}
pub unsafe fn altreal_elt(_x: Sexp, _i: isize) -> f64 {
    not_implemented("ALTREAL_ELT")
}
pub unsafe fn altcomplex_elt(_x: Sexp, _i: isize) -> Rcomplex {
    not_implemented("ALTCOMPLEX_ELT")
}
pub unsafe fn altraw_elt(_x: Sexp, _i: isize) -> u8 {
    not_implemented("ALTRAW_ELT")
}

/// Run the finalizers the last collection queued (memory.c
/// RunFinalizers).
pub unsafe fn run_finalizers() {
    let q = core().take_finalizable();
    for w in q {
        crate::extptr::run_finalizer(w);
    }
}

#[allow(dead_code)]
unsafe fn _unused() -> ! {
    error_str_owned(String::new())
}
