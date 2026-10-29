//! coerce.c. Scalar coercions of numbers are ported; whole-vector
//! coercion and string parsing/formatting go to the host's port of
//! coerce.c (robigo-runtime's `coerce.rs`) until G3.

use crate::errors::error_str;
use crate::memory::*;
use crate::state::*;
use crate::util::*;
use robigo_core::types::*;
use robigo_core::{NA_REAL, Rcomplex, Sexp};
use std::ffi::{c_char, c_int, c_uint};

const NA_INT: i32 = i32::MIN;
const WARN_NA: u32 = 1;
const WARN_INT_NA: u32 = 2;
const WARN_IMAG: u32 = 4;

unsafe fn coercion_warning(warn: u32) {
    if warn & WARN_NA != 0 {
        crate::errors::Rf_warning(c"NAs introduced by coercion".as_ptr());
    }
    if warn & WARN_INT_NA != 0 {
        crate::errors::Rf_warning(c"NAs introduced by coercion to integer range".as_ptr());
    }
    if warn & WARN_IMAG != 0 {
        crate::errors::Rf_warning(c"imaginary parts discarded in coercion".as_ptr());
    }
}

fn int_from_real(x: f64, warn: &mut u32) -> i32 {
    if x.is_nan() {
        NA_INT
    } else if x >= i32::MAX as f64 + 1.0 || x <= i32::MIN as f64 {
        *warn |= WARN_INT_NA;
        NA_INT
    } else {
        x as i32
    }
}

fn int_from_complex(x: Rcomplex, warn: &mut u32) -> i32 {
    if x.r.is_nan() || x.i.is_nan() {
        return NA_INT;
    }
    if x.r >= i32::MAX as f64 + 1.0 || x.r <= i32::MIN as f64 {
        *warn |= WARN_INT_NA;
        return NA_INT;
    }
    if x.i != 0.0 {
        *warn |= WARN_IMAG;
    }
    x.r as i32
}

/// A length-one coercion of a CHARSXP through the host (string parsing
/// and its warnings are coerce.c's String2Real & co.).
unsafe fn from_string(c: Sexp, t: u8) -> Sexp {
    let s = Rf_protect(crate::strings::Rf_ScalarString(c));
    let r = Rf_coerceVector(s, t as c_uint);
    Rf_unprotect(1);
    r
}

// R: coerce.c:asLogical2
pub unsafe fn as_logical2(x: Sexp, checking: bool, call: Sexp) -> c_int {
    if Rf_isVectorAtomic(x) != 0 {
        let n = XLENGTH(x);
        if n < 1 {
            return NA_INT;
        }
        if checking && n > 1 {
            crate::errors::errorcall_owned(call, format!("'length = {n}' in coercion to 'logical(1)'"))
        }
        match x.typ() {
            LGLSXP => LOGICAL_ELT(x, 0),
            INTSXP => {
                let v = INTEGER_ELT(x, 0);
                if v == NA_INT { NA_INT } else { (v != 0) as c_int }
            }
            REALSXP => {
                let v = REAL_ELT(x, 0);
                if v.is_nan() { NA_INT } else { (v != 0.0) as c_int }
            }
            CPLXSXP => {
                let v = COMPLEX_ELT(x, 0);
                if v.r.is_nan() || v.i.is_nan() { NA_INT } else { (v.r != 0.0 || v.i != 0.0) as c_int }
            }
            STRSXP => *LOGICAL(from_string(STRING_ELT(x, 0), LGLSXP)),
            RAWSXP => (RAW_ELT(x, 0) != 0) as c_int,
            _ => unimplemented_type("asLogical", x),
        }
    } else if x.typ() == CHARSXP {
        *LOGICAL(from_string(x, LGLSXP))
    } else {
        NA_INT
    }
}

unsafe fn unimplemented_type(s: &str, x: Sexp) -> ! {
    crate::errors::error_str_owned(format!("unimplemented type '{}' in '{}'\n", type_to_char(x), s))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asLogical(x: Sexp) -> c_int {
    as_logical2(x, false, R_NilValue)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asLogical2(x: Sexp, checking: c_int, call: Sexp) -> c_int {
    as_logical2(x, checking != 0, call)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asBool(x: Sexp) -> bool {
    let a = as_logical2(x, true, R_NilValue);
    if a == NA_INT {
        error_str("NA in coercion to boolean")
    }
    a != 0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asRboolean(x: Sexp) -> c_int {
    Rf_asBool(x) as c_int
}

// R: coerce.c:asInteger
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asInteger(x: Sexp) -> c_int {
    let mut warn = 0;
    if Rf_isVectorAtomic(x) != 0 && XLENGTH(x) >= 1 {
        match x.typ() {
            RAWSXP => RAW_ELT(x, 0) as c_int,
            LGLSXP => LOGICAL_ELT(x, 0),
            INTSXP => INTEGER_ELT(x, 0),
            REALSXP => {
                let r = int_from_real(REAL_ELT(x, 0), &mut warn);
                coercion_warning(warn);
                r
            }
            CPLXSXP => {
                let r = int_from_complex(COMPLEX_ELT(x, 0), &mut warn);
                coercion_warning(warn);
                r
            }
            STRSXP => *INTEGER(from_string(STRING_ELT(x, 0), INTSXP)),
            _ => unimplemented_type("asInteger", x),
        }
    } else if x.typ() == CHARSXP {
        *INTEGER(from_string(x, INTSXP))
    } else {
        NA_INT
    }
}

// R: coerce.c:asReal
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asReal(x: Sexp) -> f64 {
    if Rf_isVectorAtomic(x) != 0 && XLENGTH(x) >= 1 {
        match x.typ() {
            LGLSXP | INTSXP => {
                let v = INTEGER_ELT(x, 0);
                if v == NA_INT { NA_REAL } else { v as f64 }
            }
            REALSXP => REAL_ELT(x, 0),
            CPLXSXP => {
                let z = COMPLEX_ELT(x, 0);
                if z.r.is_nan() || z.i.is_nan() {
                    return NA_REAL;
                }
                if z.i != 0.0 {
                    coercion_warning(WARN_IMAG);
                }
                z.r
            }
            STRSXP => *REAL(from_string(STRING_ELT(x, 0), REALSXP)),
            RAWSXP => RAW_ELT(x, 0) as f64,
            _ => unimplemented_type("asReal", x),
        }
    } else if x.typ() == CHARSXP {
        *REAL(from_string(x, REALSXP))
    } else {
        NA_REAL
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asComplex(x: Sexp) -> Rcomplex {
    if Rf_isVectorAtomic(x) != 0 && XLENGTH(x) >= 1 {
        if x.typ() == CPLXSXP {
            return COMPLEX_ELT(x, 0);
        }
        let s = if x.typ() == STRSXP {
            from_string(STRING_ELT(x, 0), CPLXSXP)
        } else {
            Rf_coerceVector(x, CPLXSXP as c_uint)
        };
        return *COMPLEX(s);
    } else if x.typ() == CHARSXP {
        return *COMPLEX(from_string(x, CPLXSXP));
    }
    Rcomplex { r: NA_REAL, i: NA_REAL }
}

// R: coerce.c:asChar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asChar(x: Sexp) -> Sexp {
    if Rf_isVectorAtomic(x) != 0 && XLENGTH(x) >= 1 {
        match x.typ() {
            LGLSXP => {
                let v = LOGICAL_ELT(x, 0);
                if v == NA_INT {
                    return R_NaString;
                }
                core().mk_char(if v != 0 { b"TRUE" } else { b"FALSE" }, 0)
            }
            INTSXP => {
                let v = INTEGER_ELT(x, 0);
                if v == NA_INT {
                    return R_NaString;
                }
                core().mk_char(v.to_string().as_bytes(), 0)
            }
            REALSXP | CPLXSXP => crate::host::op("asChar\0", &[x]),
            STRSXP => STRING_ELT(x, 0),
            _ => R_NaString,
        }
    } else if x.typ() == CHARSXP {
        x
    } else if x.typ() == SYMSXP {
        x.car()
    } else {
        R_NaString
    }
}

// R: coerce.c:coerceVector (the host's port)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_coerceVector(v: Sexp, t: c_uint) -> Sexp {
    if v.typ() as c_uint == t {
        return v;
    }
    // R: coerce.c:coerceSymbol, coercePairList — a symbol, call or
    // pairlist becomes a one-element expression vector holding it
    if t == EXPRSXP as c_uint && matches!(v.typ(), SYMSXP | LANGSXP | LISTSXP) {
        let r = core().alloc_vector(EXPRSXP, 1);
        SET_VECTOR_ELT(r, 0, v);
        return r;
    }
    let ts = Rf_protect(core().alloc_vector(INTSXP, 1));
    *(ts.std_data() as *mut i32) = t as i32;
    let r = crate::host::op("coerceVector\0", &[v, ts]);
    Rf_unprotect(1);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asCharacterFactor(x: Sexp) -> Sexp {
    crate::host::op("asCharacterFactor\0", &[x])
}

// R: coerce.c:PairToVectorList
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_PairToVectorList(x: Sexp) -> Sexp {
    let mut len = 0;
    let mut named = false;
    let mut p = x;
    while p != R_NilValue && !p.is_null() {
        named |= p.tag() != R_NilValue;
        len += 1;
        p = p.cdr();
    }
    Rf_protect(x);
    let xnew = Rf_protect(core().alloc_vector(VECSXP, len));
    let mut p = x;
    let mut i = 0;
    while i < len {
        if MAYBE_REFERENCED(x) != 0 {
            ENSURE_NAMEDMAX(p.car());
        }
        SET_VECTOR_ELT(xnew, i as isize, p.car());
        p = p.cdr();
        i += 1;
    }
    if named {
        let xnames = Rf_protect(core().alloc_vector(STRSXP, len));
        let mut p = x;
        for i in 0..len {
            if p.tag() == R_NilValue {
                SET_STRING_ELT(xnames, i as isize, R_BlankString);
            } else {
                SET_STRING_ELT(xnames, i as isize, p.tag().car());
            }
            p = p.cdr();
        }
        crate::attrib::Rf_setAttrib(xnew, R_NamesSymbol, xnames);
        Rf_unprotect(1);
    }
    Rf_copyMostAttrib(x, xnew);
    Rf_unprotect(2);
    xnew
}

// R: coerce.c:VectorToPairList
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_VectorToPairList(x: Sexp) -> Sexp {
    Rf_protect(x);
    let len = Rf_length(x);
    let xnew = Rf_protect(crate::memory::Rf_allocList(len));
    let xnames = Rf_protect(crate::attrib::Rf_getAttrib(x, R_NamesSymbol));
    let named = xnames != R_NilValue;
    let mut xptr = xnew;
    for i in 0..len as isize {
        if MAYBE_REFERENCED(x) != 0 {
            ENSURE_NAMEDMAX(VECTOR_ELT(x, i));
        }
        SETCAR(xptr, VECTOR_ELT(x, i));
        if named && !STRING_ELT(xnames, i).char_bytes().is_empty() {
            SET_TAG(xptr, crate::strings::Rf_installTrChar(STRING_ELT(xnames, i)));
        }
        xptr = xptr.cdr();
    }
    if len > 0 {
        Rf_copyMostAttrib(x, xnew);
    }
    Rf_unprotect(3);
    xnew
}

use crate::attrib::Rf_copyMostAttrib;

#[allow(dead_code)]
fn _c(_: *const c_char) {}
