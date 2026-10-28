//! Rinlinedfuns.h's constructors (lists, calls, scalars), list.c and
//! builtin.c's lengthgets.

use crate::errors::{error_str, error_str_owned};
use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Rcomplex, Sexp};
use std::ffi::{CStr, c_char, c_int};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_list1(s: Sexp) -> Sexp {
    Rf_cons(s, R_NilValue)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_list2(s: Sexp, t: Sexp) -> Sexp {
    Rf_protect(s);
    let r = Rf_cons(s, Rf_list1(t));
    Rf_unprotect(1);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_list3(s: Sexp, t: Sexp, u: Sexp) -> Sexp {
    Rf_protect(s);
    let r = Rf_cons(s, Rf_list2(t, u));
    Rf_unprotect(1);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_list4(s: Sexp, t: Sexp, u: Sexp, v: Sexp) -> Sexp {
    Rf_protect(s);
    let r = Rf_cons(s, Rf_list3(t, u, v));
    Rf_unprotect(1);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_list5(s: Sexp, t: Sexp, u: Sexp, v: Sexp, w: Sexp) -> Sexp {
    Rf_protect(s);
    let r = Rf_cons(s, Rf_list4(t, u, v, w));
    Rf_unprotect(1);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_list6(s: Sexp, t: Sexp, u: Sexp, v: Sexp, w: Sexp, x: Sexp) -> Sexp {
    Rf_protect(s);
    let r = Rf_cons(s, Rf_list5(t, u, v, w, x));
    Rf_unprotect(1);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lang1(s: Sexp) -> Sexp {
    Rf_lcons(s, R_NilValue)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lang2(s: Sexp, t: Sexp) -> Sexp {
    Rf_protect(s);
    let r = Rf_lcons(s, Rf_list1(t));
    Rf_unprotect(1);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lang3(s: Sexp, t: Sexp, u: Sexp) -> Sexp {
    Rf_protect(s);
    let r = Rf_lcons(s, Rf_list2(t, u));
    Rf_unprotect(1);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lang4(s: Sexp, t: Sexp, u: Sexp, v: Sexp) -> Sexp {
    Rf_protect(s);
    let r = Rf_lcons(s, Rf_list3(t, u, v));
    Rf_unprotect(1);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lang5(s: Sexp, t: Sexp, u: Sexp, v: Sexp, w: Sexp) -> Sexp {
    Rf_protect(s);
    let r = Rf_lcons(s, Rf_list4(t, u, v, w));
    Rf_unprotect(1);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lang6(s: Sexp, t: Sexp, u: Sexp, v: Sexp, w: Sexp, x: Sexp) -> Sexp {
    Rf_protect(s);
    let r = Rf_lcons(s, Rf_list5(t, u, v, w, x));
    Rf_unprotect(1);
    r
}

// R: list.c / Rinlinedfuns.h
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_elt(list: Sexp, i: c_int) -> Sexp {
    let mut r = list;
    if i < 0 || i > Rf_length(list) {
        return R_NilValue;
    }
    for _ in 0..i {
        r = r.cdr();
    }
    r.car()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lastElt(s: Sexp) -> Sexp {
    let mut s = s;
    if s != R_NilValue {
        while s.cdr() != R_NilValue {
            s = s.cdr();
        }
    }
    s
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_listAppend(s: Sexp, t: Sexp) -> Sexp {
    if s == R_NilValue {
        return t;
    }
    let mut r = s;
    while r.cdr() != R_NilValue {
        r = r.cdr();
    }
    SETCDR(r, t);
    s
}

// R: Rinlinedfuns.h:nthcdr
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_nthcdr(s: Sexp, n: c_int) -> Sexp {
    if Rf_isList(s) != 0 || Rf_isLanguage(s) != 0 || crate::util::Rf_isFrame(s) != 0 || s.typ() == DOTSXP {
        let mut s = s;
        let mut n = n;
        while n > 0 {
            if s == R_NilValue {
                error_str("'nthcdr' list shorter than n")
            }
            s = s.cdr();
            n -= 1;
        }
        s
    } else {
        error_str("'nthcdr' needs a list to CDR down")
    }
}

use crate::util::Rf_isLanguage;
use crate::util::Rf_isList;

// R: Rinlinedfuns.h:mkNamed — stops at the first empty name
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkNamed(t: c_int, names: *const *const c_char) -> Sexp {
    let mut n = 0;
    while *(*names.add(n)) != 0 {
        n += 1;
    }
    let ans = Rf_protect(alloc_vector(t as u32, n as isize));
    let nms = Rf_protect(core().alloc_vector(STRSXP, n));
    for i in 0..n {
        SET_STRING_ELT(nms, i as isize, crate::strings::Rf_mkChar(*names.add(i)));
    }
    crate::attrib::Rf_setAttrib(ans, R_NamesSymbol, nms);
    Rf_unprotect(2);
    ans
}

// ---- scalars ---------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ScalarInteger(x: c_int) -> Sexp {
    let a = core().alloc_vector(INTSXP, 1);
    *(a.std_data() as *mut c_int) = x;
    a
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ScalarReal(x: f64) -> Sexp {
    let a = core().alloc_vector(REALSXP, 1);
    *(a.std_data() as *mut f64) = x;
    a
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ScalarLogical(x: c_int) -> Sexp {
    if x == i32::MIN {
        R_LogicalNAValue
    } else if x != 0 {
        R_TrueValue
    } else {
        R_FalseValue
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ScalarRaw(x: u8) -> Sexp {
    let a = core().alloc_vector(RAWSXP, 1);
    *a.std_data() = x;
    a
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ScalarComplex(x: Rcomplex) -> Sexp {
    let a = core().alloc_vector(CPLXSXP, 1);
    *(a.std_data() as *mut Rcomplex) = x;
    a
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkTrue() -> Sexp {
    let a = core().alloc_vector(LGLSXP, 1);
    *(a.std_data() as *mut c_int) = 1;
    a
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkFalse() -> Sexp {
    let a = core().alloc_vector(LGLSXP, 1);
    *(a.std_data() as *mut c_int) = 0;
    a
}

// ---- lengthgets (builtin.c) ----------------------------------------------------

// R: builtin.c:xlengthgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_xlengthgets(x: Sexp, len: isize) -> Sexp {
    if !crate::util::is_vector(x) && Rf_isList(x) == 0 {
        error_str("cannot set length of non-(vector or list)")
    }
    if len < 0 {
        error_str("invalid value")
    }
    if x == R_NilValue && len > 0 {
        error_str("cannot set length of NULL")
    }
    if x == R_NilValue {
        return R_NilValue;
    }
    Rf_protect(x);
    let xnames = crate::attrib::Rf_getAttrib(x, R_NamesSymbol);
    let t = x.typ();
    let lenx = Rf_xlength(x);
    let rval = Rf_protect(alloc_vector(t as u32, len));
    let names = if xnames != R_NilValue { Rf_protect(core().alloc_vector(STRSXP, len as usize)) } else { R_NilValue };
    for i in 0..len {
        let have = i < lenx;
        match t {
            NILSXP => {}
            LGLSXP | INTSXP => *INTEGER(rval).offset(i) = if have { *INTEGER(x).offset(i) } else { i32::MIN },
            REALSXP => *REAL(rval).offset(i) = if have { *REAL(x).offset(i) } else { robigo_core::NA_REAL },
            CPLXSXP => {
                *COMPLEX(rval).offset(i) = if have {
                    *COMPLEX(x).offset(i)
                } else {
                    Rcomplex { r: robigo_core::NA_REAL, i: robigo_core::NA_REAL }
                }
            }
            STRSXP => SET_STRING_ELT(rval, i, if have { STRING_ELT(x, i) } else { R_NaString }),
            VECSXP | EXPRSXP => {
                if have {
                    SET_VECTOR_ELT(rval, i, VECTOR_ELT(x, i));
                }
            }
            RAWSXP => *RAW(rval).offset(i) = if have { *RAW(x).offset(i) } else { 0 },
            LISTSXP => {}
            _ => {
                error_str_owned(format!("unimplemented type '{}' in '{}'\n", crate::util::type_name(t), "xlengthgets"))
            }
        }
        if names != R_NilValue {
            SET_STRING_ELT(names, i, if have { STRING_ELT(xnames, i) } else { R_BlankString });
        }
    }
    if t == LISTSXP {
        let mut s = rval;
        let mut p = x;
        while s != R_NilValue {
            if p != R_NilValue {
                SETCAR(s, p.car());
                SET_TAG(s, p.tag());
                p = p.cdr();
            }
            s = s.cdr();
        }
    }
    if names != R_NilValue {
        crate::attrib::Rf_setAttrib(rval, R_NamesSymbol, names);
        Rf_unprotect(1);
    }
    Rf_unprotect(2);
    rval
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lengthgets(x: Sexp, len: c_int) -> Sexp {
    Rf_xlengthgets(x, len as isize)
}

#[allow(dead_code)]
unsafe fn cstr(p: *const c_char) -> &'static [u8] {
    CStr::from_ptr(p).to_bytes()
}
