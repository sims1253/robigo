//! Rinlinedfuns.h's constructors (lists, calls, scalars), list.c and
//! builtin.c's lengthgets.

use crate::errors::{Res, error_str, ret};
use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Rcomplex, Sexp};
use std::ffi::{c_char, c_int};

/// `CONS(s, rest())`, `s` protected meanwhile.
#[inline(always)]
unsafe fn cons_with(s: Sexp, rest: impl FnOnce() -> Res<Sexp>, lang: bool) -> Res<Sexp> {
    protect(s)?;
    let t = rest()?;
    let r = if lang { Rf_lcons(s, t) } else { Rf_cons(s, t) };
    unprotect(1)?;
    Ok(r)
}

/// list2.
#[inline]
pub unsafe fn list2(s: Sexp, t: Sexp) -> Res<Sexp> {
    cons_with(s, || Ok(Rf_list1(t)), false)
}

/// list3.
#[inline]
pub unsafe fn list3(s: Sexp, t: Sexp, u: Sexp) -> Res<Sexp> {
    cons_with(s, || list2(t, u), false)
}

/// list4.
pub unsafe fn list4(s: Sexp, t: Sexp, u: Sexp, v: Sexp) -> Res<Sexp> {
    cons_with(s, || list3(t, u, v), false)
}

/// list5.
pub unsafe fn list5(s: Sexp, t: Sexp, u: Sexp, v: Sexp, w: Sexp) -> Res<Sexp> {
    cons_with(s, || list4(t, u, v, w), false)
}

/// lang2.
#[inline]
pub unsafe fn lang2(s: Sexp, t: Sexp) -> Res<Sexp> {
    cons_with(s, || Ok(Rf_list1(t)), true)
}

/// lang3.
#[inline]
pub unsafe fn lang3(s: Sexp, t: Sexp, u: Sexp) -> Res<Sexp> {
    cons_with(s, || list2(t, u), true)
}

/// lang4.
pub unsafe fn lang4(s: Sexp, t: Sexp, u: Sexp, v: Sexp) -> Res<Sexp> {
    cons_with(s, || list3(t, u, v), true)
}

// The exports build on each other as GNU R's inline functions do (an
// export's body may call exports: its frame is a wrapper's); Rust code
// calls the `Res` versions above.
macro_rules! cons_exports {
    ($($name:ident($first:ident $(, $rest:ident)*) = $cons:ident, $tail:ident;)*) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name($first: Sexp $(, $rest: Sexp)*) -> Sexp {
                Rf_protect($first);
                let r = $cons($first, $tail($($rest),*));
                Rf_unprotect(1);
                r
            }
        )*
    };
}

cons_exports! {
    Rf_list2(s, t) = Rf_cons, Rf_list1;
    Rf_list3(s, t, u) = Rf_cons, Rf_list2;
    Rf_list4(s, t, u, v) = Rf_cons, Rf_list3;
    Rf_list5(s, t, u, v, w) = Rf_cons, Rf_list4;
    Rf_list6(s, t, u, v, w, x) = Rf_cons, Rf_list5;
    Rf_lang2(s, t) = Rf_lcons, Rf_list1;
    Rf_lang3(s, t, u) = Rf_lcons, Rf_list2;
    Rf_lang4(s, t, u, v) = Rf_lcons, Rf_list3;
    Rf_lang5(s, t, u, v, w) = Rf_lcons, Rf_list4;
    Rf_lang6(s, t, u, v, w, x) = Rf_lcons, Rf_list5;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_list1(s: Sexp) -> Sexp {
    Rf_cons(s, R_NilValue)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lang1(s: Sexp) -> Sexp {
    Rf_lcons(s, R_NilValue)
}

// R: list.c / Rinlinedfuns.h
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_elt(list: Sexp, i: c_int) -> Sexp {
    let mut r = list;
    if i < 0 || i > ret(r_length(list)) {
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
    ret(list_append(s, t))
}

/// listAppend.
pub unsafe fn list_append(s: Sexp, t: Sexp) -> Res<Sexp> {
    if s == R_NilValue {
        return Ok(t);
    }
    let mut r = s;
    while r.cdr() != R_NilValue {
        r = r.cdr();
    }
    setcdr(r, t)?;
    Ok(s)
}

// R: Rinlinedfuns.h:nthcdr
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_nthcdr(s: Sexp, n: c_int) -> Sexp {
    ret(nthcdr(s, n))
}

unsafe fn nthcdr(s: Sexp, n: c_int) -> Res<Sexp> {
    if Rf_isList(s) != 0 || Rf_isLanguage(s) != 0 || crate::util::Rf_isFrame(s) != 0 || s.typ() == DOTSXP {
        let mut s = s;
        let mut n = n;
        while n > 0 {
            if s == R_NilValue {
                return Err(error_str("'nthcdr' list shorter than n"));
            }
            s = s.cdr();
            n -= 1;
        }
        Ok(s)
    } else {
        Err(error_str("'nthcdr' needs a list to CDR down"))
    }
}

use crate::util::Rf_isLanguage;
use crate::util::Rf_isList;

// R: Rinlinedfuns.h:mkNamed — stops at the first empty name
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mkNamed(t: c_int, names: *const *const c_char) -> Sexp {
    ret(mk_named(t, names))
}

unsafe fn mk_named(t: c_int, names: *const *const c_char) -> Res<Sexp> {
    let mut n = 0;
    while *(*names.add(n)) != 0 {
        n += 1;
    }
    let ans = protect(alloc_vector(t as u32, n as isize)?)?;
    let nms = protect(try_alloc_vector(STRSXP, n)?)?;
    for i in 0..n {
        set_string_elt(nms, i as isize, crate::strings::mk_char(*names.add(i))?)?;
    }
    crate::attrib::set_attrib(ans, R_NamesSymbol, nms)?;
    unprotect(2)?;
    Ok(ans)
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
    ret(xlengthgets(x, len))
}

/// xlengthgets.
pub unsafe fn xlengthgets(x: Sexp, len: isize) -> Res<Sexp> {
    if !crate::util::is_vector(x) && Rf_isList(x) == 0 {
        return Err(error_str("cannot set length of non-(vector or list)"));
    }
    if len < 0 {
        return Err(error_str("invalid value"));
    }
    if x == R_NilValue && len > 0 {
        return Err(error_str("cannot set length of NULL"));
    }
    if x == R_NilValue {
        return Ok(R_NilValue);
    }
    protect(x)?;
    let xnames = crate::attrib::get_attrib(x, R_NamesSymbol)?;
    let t = x.typ();
    let lenx = r_xlength(x)?;
    let rval = protect(alloc_vector(t as u32, len)?)?;
    let names = if xnames != R_NilValue { protect(try_alloc_vector(STRSXP, len as usize)?)? } else { R_NilValue };
    for i in 0..len {
        let have = i < lenx;
        match t {
            NILSXP => {}
            LGLSXP | INTSXP => *integer(rval)?.offset(i) = if have { *integer(x)?.offset(i) } else { i32::MIN },
            REALSXP => *real(rval)?.offset(i) = if have { *real(x)?.offset(i) } else { robigo_core::NA_REAL },
            CPLXSXP => {
                *complex(rval)?.offset(i) = if have {
                    *complex(x)?.offset(i)
                } else {
                    Rcomplex { r: robigo_core::NA_REAL, i: robigo_core::NA_REAL }
                }
            }
            STRSXP => set_string_elt(rval, i, if have { string_elt(x, i)? } else { R_NaString })?,
            VECSXP | EXPRSXP => {
                if have {
                    set_vector_elt(rval, i, vector_elt(x, i)?)?;
                }
            }
            RAWSXP => *raw(rval)?.offset(i) = if have { *raw(x)?.offset(i) } else { 0 },
            LISTSXP => {}
            _ => {
                return Err(error_str(&format!(
                    "unimplemented type '{}' in '{}'\n",
                    crate::util::type_name(t),
                    "xlengthgets"
                )));
            }
        }
        if names != R_NilValue {
            set_string_elt(names, i, if have { string_elt(xnames, i)? } else { R_BlankString })?;
        }
    }
    if t == LISTSXP {
        let mut s = rval;
        let mut p = x;
        while s != R_NilValue {
            if p != R_NilValue {
                setcar(s, p.car())?;
                set_tag(s, p.tag())?;
                p = p.cdr();
            }
            s = s.cdr();
        }
    }
    if names != R_NilValue {
        crate::attrib::set_attrib(rval, R_NamesSymbol, names)?;
        unprotect(1)?;
    }
    unprotect(2)?;
    Ok(rval)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lengthgets(x: Sexp, len: c_int) -> Sexp {
    ret(xlengthgets(x, len as isize))
}

/// lengthgets.
pub unsafe fn lengthgets(x: Sexp, len: c_int) -> Res<Sexp> {
    xlengthgets(x, len as isize)
}
