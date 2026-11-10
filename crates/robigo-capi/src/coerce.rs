//! coerce.c. Scalar coercions of numbers are ported; whole-vector
//! coercion and string parsing/formatting go to the host's port of
//! coerce.c (robigo-runtime's `coerce.rs`) until G3.

use crate::errors::{Res, error_str, ret, warning_str};
use crate::memory::*;
use crate::state::*;
use crate::util::*;
use robigo_core::types::*;
use robigo_core::{NA_REAL, Rcomplex, Sexp};
use std::ffi::{c_int, c_uint};

const NA_INT: i32 = i32::MIN;
const WARN_NA: u32 = 1;
const WARN_INT_NA: u32 = 2;
const WARN_IMAG: u32 = 4;

unsafe fn coercion_warning(warn: u32) -> Res<()> {
    if warn & WARN_NA != 0 {
        warning_str("NAs introduced by coercion")?;
    }
    if warn & WARN_INT_NA != 0 {
        warning_str("NAs introduced by coercion to integer range")?;
    }
    if warn & WARN_IMAG != 0 {
        warning_str("imaginary parts discarded in coercion")?;
    }
    Ok(())
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
unsafe fn from_string(c: Sexp, t: u8) -> Res<Sexp> {
    let s = protect(crate::strings::scalar_string(c)?)?;
    let r = coerce_vector(s, t as c_uint)?;
    unprotect(1)?;
    Ok(r)
}

// R: coerce.c:asLogical2
pub unsafe fn as_logical2(x: Sexp, checking: bool, call: Sexp) -> Res<c_int> {
    if Rf_isVectorAtomic(x) != 0 {
        let n = checked_xlength(x)?;
        if n < 1 {
            return Ok(NA_INT);
        }
        if checking && n > 1 {
            return Err(crate::errors::errorcall_str(call, &format!("'length = {n}' in coercion to 'logical(1)'")));
        }
        Ok(match x.typ() {
            LGLSXP => logical_elt(x, 0)?,
            INTSXP => {
                let v = integer_elt(x, 0)?;
                if v == NA_INT { NA_INT } else { (v != 0) as c_int }
            }
            REALSXP => {
                let v = real_elt(x, 0)?;
                if v.is_nan() { NA_INT } else { (v != 0.0) as c_int }
            }
            CPLXSXP => {
                let v = complex_elt(x, 0)?;
                if v.r.is_nan() || v.i.is_nan() { NA_INT } else { (v.r != 0.0 || v.i != 0.0) as c_int }
            }
            STRSXP => *logical(from_string(string_elt(x, 0)?, LGLSXP)?)?,
            RAWSXP => (raw_elt(x, 0)? != 0) as c_int,
            _ => return Err(unimplemented_type("asLogical", x)),
        })
    } else if x.typ() == CHARSXP {
        Ok(*logical(from_string(x, LGLSXP)?)?)
    } else {
        Ok(NA_INT)
    }
}

unsafe fn unimplemented_type(s: &str, x: Sexp) -> crate::errors::Unwind {
    crate::errors::error_str(&format!("unimplemented type '{}' in '{}'\n", type_to_char(x), s))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asLogical(x: Sexp) -> c_int {
    ret(as_logical2(x, false, R_NilValue))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asLogical2(x: Sexp, checking: c_int, call: Sexp) -> c_int {
    ret(as_logical2(x, checking != 0, call))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asBool(x: Sexp) -> bool {
    ret(as_bool(x))
}

/// asBool.
pub unsafe fn as_bool(x: Sexp) -> Res<bool> {
    let a = as_logical2(x, true, R_NilValue)?;
    if a == NA_INT {
        return Err(error_str("NA in coercion to boolean"));
    }
    Ok(a != 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asRboolean(x: Sexp) -> c_int {
    ret(as_bool(x)) as c_int
}

// R: coerce.c:asInteger
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asInteger(x: Sexp) -> c_int {
    ret(as_integer(x))
}

/// asInteger.
pub unsafe fn as_integer(x: Sexp) -> Res<c_int> {
    let mut warn = 0;
    if Rf_isVectorAtomic(x) != 0 && checked_xlength(x)? >= 1 {
        Ok(match x.typ() {
            RAWSXP => raw_elt(x, 0)? as c_int,
            LGLSXP => logical_elt(x, 0)?,
            INTSXP => integer_elt(x, 0)?,
            REALSXP => {
                let r = int_from_real(real_elt(x, 0)?, &mut warn);
                coercion_warning(warn)?;
                r
            }
            CPLXSXP => {
                let r = int_from_complex(complex_elt(x, 0)?, &mut warn);
                coercion_warning(warn)?;
                r
            }
            STRSXP => *integer(from_string(string_elt(x, 0)?, INTSXP)?)?,
            _ => return Err(unimplemented_type("asInteger", x)),
        })
    } else if x.typ() == CHARSXP {
        Ok(*integer(from_string(x, INTSXP)?)?)
    } else {
        Ok(NA_INT)
    }
}

// R: coerce.c:asReal
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asReal(x: Sexp) -> f64 {
    ret(as_real(x))
}

/// asReal.
pub unsafe fn as_real(x: Sexp) -> Res<f64> {
    if Rf_isVectorAtomic(x) != 0 && checked_xlength(x)? >= 1 {
        Ok(match x.typ() {
            LGLSXP | INTSXP => {
                let v = integer_elt(x, 0)?;
                if v == NA_INT { NA_REAL } else { v as f64 }
            }
            REALSXP => real_elt(x, 0)?,
            CPLXSXP => {
                let z = complex_elt(x, 0)?;
                if z.r.is_nan() || z.i.is_nan() {
                    return Ok(NA_REAL);
                }
                if z.i != 0.0 {
                    coercion_warning(WARN_IMAG)?;
                }
                z.r
            }
            STRSXP => *real(from_string(string_elt(x, 0)?, REALSXP)?)?,
            RAWSXP => raw_elt(x, 0)? as f64,
            _ => return Err(unimplemented_type("asReal", x)),
        })
    } else if x.typ() == CHARSXP {
        Ok(*real(from_string(x, REALSXP)?)?)
    } else {
        Ok(NA_REAL)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asComplex(x: Sexp) -> Rcomplex {
    ret(as_complex(x))
}

unsafe fn as_complex(x: Sexp) -> Res<Rcomplex> {
    if Rf_isVectorAtomic(x) != 0 && checked_xlength(x)? >= 1 {
        if x.typ() == CPLXSXP {
            return complex_elt(x, 0);
        }
        let s = if x.typ() == STRSXP {
            from_string(string_elt(x, 0)?, CPLXSXP)?
        } else {
            coerce_vector(x, CPLXSXP as c_uint)?
        };
        return Ok(*complex(s)?);
    } else if x.typ() == CHARSXP {
        return Ok(*complex(from_string(x, CPLXSXP)?)?);
    }
    Ok(Rcomplex { r: NA_REAL, i: NA_REAL })
}

// R: coerce.c:asChar
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asChar(x: Sexp) -> Sexp {
    ret(as_char(x))
}

/// asChar.
pub unsafe fn as_char(x: Sexp) -> Res<Sexp> {
    if Rf_isVectorAtomic(x) != 0 && checked_xlength(x)? >= 1 {
        Ok(match x.typ() {
            LGLSXP => {
                let v = logical_elt(x, 0)?;
                if v == NA_INT {
                    return Ok(R_NaString);
                }
                core().mk_char(if v != 0 { b"TRUE" } else { b"FALSE" }, 0)
            }
            INTSXP => {
                let v = integer_elt(x, 0)?;
                if v == NA_INT {
                    return Ok(R_NaString);
                }
                core().mk_char(v.to_string().as_bytes(), 0)
            }
            REALSXP | CPLXSXP => crate::host::op("asChar\0", &[x])?,
            STRSXP => string_elt(x, 0)?,
            _ => R_NaString,
        })
    } else if x.typ() == CHARSXP {
        Ok(x)
    } else if x.typ() == SYMSXP {
        Ok(x.car())
    } else {
        Ok(R_NaString)
    }
}

// R: coerce.c:coerceVector (the host's port)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_coerceVector(v: Sexp, t: c_uint) -> Sexp {
    ret(coerce_vector(v, t))
}

/// coerceVector.
pub unsafe fn coerce_vector(v: Sexp, t: c_uint) -> Res<Sexp> {
    if v.typ() as c_uint == t {
        return Ok(v);
    }
    // R: coerce.c:coerceSymbol, coercePairList — a symbol, call or
    // pairlist becomes a one-element expression vector holding it
    if t == EXPRSXP as c_uint && matches!(v.typ(), SYMSXP | LANGSXP | LISTSXP) {
        let r = core().alloc_vector(EXPRSXP, 1);
        set_vector_elt(r, 0, v)?;
        return Ok(r);
    }
    // R: coerce.c:coercePairList (PairToVectorList) and coerceToPairList:
    // pairlist <-> list conversion moves the elements themselves, never
    // copies. The host's round trip through runtime values rebuilt every
    // element, attributes included, which C can tell (rlang 1.1's
    // is_splice_box compares the attribute pairlist by address, and
    // dots_values coerces its captured pairlist here).
    if t == VECSXP as c_uint && matches!(v.typ(), LISTSXP | LANGSXP | DOTSXP) {
        return pair_to_vector_list(v);
    }
    if t == LISTSXP as c_uint && matches!(v.typ(), VECSXP | EXPRSXP) {
        return vector_to_pair_list(v);
    }
    let ts = protect(core().alloc_vector(INTSXP, 1))?;
    *(ts.std_data() as *mut i32) = t as i32;
    let r = crate::host::op("coerceVector\0", &[v, ts])?;
    unprotect(1)?;
    Ok(r)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_asCharacterFactor(x: Sexp) -> Sexp {
    ret(as_character_factor(x))
}

/// asCharacterFactor.
pub unsafe fn as_character_factor(x: Sexp) -> Res<Sexp> {
    crate::host::op("asCharacterFactor\0", &[x])
}

// R: coerce.c:PairToVectorList
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_PairToVectorList(x: Sexp) -> Sexp {
    ret(pair_to_vector_list(x))
}

/// PairToVectorList.
pub unsafe fn pair_to_vector_list(x: Sexp) -> Res<Sexp> {
    let mut len = 0;
    let mut named = false;
    let mut p = x;
    while p != R_NilValue && !p.is_null() {
        named |= p.tag() != R_NilValue;
        len += 1;
        p = p.cdr();
    }
    protect(x)?;
    let xnew = protect(try_alloc_vector(VECSXP, len)?)?;
    let mut p = x;
    let mut i = 0;
    while i < len {
        if MAYBE_REFERENCED(x) != 0 {
            ENSURE_NAMEDMAX(p.car());
        }
        set_vector_elt(xnew, i as isize, p.car())?;
        p = p.cdr();
        i += 1;
    }
    if named {
        let xnames = protect(try_alloc_vector(STRSXP, len)?)?;
        let mut p = x;
        for i in 0..len {
            if p.tag() == R_NilValue {
                set_string_elt(xnames, i as isize, R_BlankString)?;
            } else {
                set_string_elt(xnames, i as isize, p.tag().car())?;
            }
            p = p.cdr();
        }
        crate::attrib::set_attrib(xnew, R_NamesSymbol, xnames)?;
        unprotect(1)?;
    }
    copy_most_attrib(x, xnew)?;
    unprotect(2)?;
    Ok(xnew)
}

// R: coerce.c:VectorToPairList
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_VectorToPairList(x: Sexp) -> Sexp {
    ret(vector_to_pair_list(x))
}

/// VectorToPairList.
pub unsafe fn vector_to_pair_list(x: Sexp) -> Res<Sexp> {
    protect(x)?;
    let len = r_length(x)?;
    let xnew = protect(crate::memory::Rf_allocList(len))?;
    let xnames = protect(crate::attrib::get_attrib(x, R_NamesSymbol)?)?;
    let named = xnames != R_NilValue;
    let mut xptr = xnew;
    for i in 0..len as isize {
        if MAYBE_REFERENCED(x) != 0 {
            ENSURE_NAMEDMAX(vector_elt(x, i)?);
        }
        setcar(xptr, vector_elt(x, i)?)?;
        if named && !string_elt(xnames, i)?.char_bytes().is_empty() {
            set_tag(xptr, crate::strings::install_tr_char(string_elt(xnames, i)?)?)?;
        }
        xptr = xptr.cdr();
    }
    if len > 0 {
        copy_most_attrib(x, xnew)?;
    }
    unprotect(3)?;
    Ok(xnew)
}

use crate::attrib::copy_most_attrib;
