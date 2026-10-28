//! attrib.c: attribute access with GNU R's validation.

use crate::errors::{error_str, error_str_owned};
use crate::memory::*;
use crate::state::*;
use crate::strings::Rf_installTrChar;
use crate::util::*;
use robigo_core::types::*;
use robigo_core::{Sexp, flags};
use std::ffi::{CStr, c_char, c_int};

fn sym(name: &str) -> Sexp {
    core().install(name.as_bytes())
}

unsafe fn is_one_dimensional_array(vec: Sexp) -> bool {
    if is_vector(vec) || Rf_isList(vec) != 0 || Rf_isLanguage(vec) != 0 {
        let s = Rf_getAttrib(vec, R_DimSymbol);
        if s.typ() == INTSXP && LENGTH(s) == 1 {
            return true;
        }
    }
    false
}

/// The class attribute without the names/row.names special cases.
pub unsafe fn get_class(x: Sexp) -> Sexp {
    let mut s = ATTRIB(x);
    while s != R_NilValue {
        if s.tag() == R_ClassSymbol {
            return s.car();
        }
        s = s.cdr();
    }
    R_NilValue
}

// R: attrib.c:getAttrib0
pub unsafe fn get_attrib0(vec: Sexp, name: Sexp) -> Sexp {
    if name == R_NamesSymbol {
        if is_one_dimensional_array(vec) {
            let s = Rf_getAttrib(vec, R_DimNamesSymbol);
            if s != R_NilValue {
                MARK_NOT_MUTABLE(VECTOR_ELT(s, 0));
                return VECTOR_ELT(s, 0);
            }
        }
        if matches!(vec.typ(), LISTSXP | LANGSXP | DOTSXP | NILSXP) {
            let len = Rf_length(vec);
            let s = Rf_protect(core().alloc_vector(STRSXP, len as usize));
            let mut any = false;
            let mut v = vec;
            let mut i = 0;
            while v != R_NilValue {
                let t = v.tag();
                if t == R_NilValue {
                    SET_STRING_ELT(s, i, R_BlankString);
                } else if t.typ() == SYMSXP {
                    any = true;
                    SET_STRING_ELT(s, i, t.car());
                } else {
                    error_str_owned(format!("getAttrib: invalid type ({}) for TAG", type_to_char(t)))
                }
                v = v.cdr();
                i += 1;
            }
            Rf_unprotect(1);
            if any {
                MARK_NOT_MUTABLE(s);
                return s;
            }
            return R_NilValue;
        }
    }
    let mut s = ATTRIB(vec);
    while s != R_NilValue {
        if s.tag() == name {
            if name == R_DimNamesSymbol && s.car().typ() == LISTSXP {
                error_str("old list is no longer allowed for dimnames attribute")
            }
            MARK_NOT_MUTABLE(s.car());
            return s.car();
        }
        s = s.cdr();
    }
    R_NilValue
}

// R: attrib.c:getAttrib
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_getAttrib(vec: Sexp, name: Sexp) -> Sexp {
    if vec.typ() == CHARSXP {
        error_str("cannot have attributes on a CHARSXP")
    }
    if ATTRIB(vec) == R_NilValue && !matches!(vec.typ(), LISTSXP | LANGSXP | DOTSXP) {
        return R_NilValue;
    }
    let name = if name.typ() == STRSXP { Rf_installTrChar(STRING_ELT(name, 0)) } else { name };
    if name == R_RowNamesSymbol {
        let s = get_attrib0(vec, R_RowNamesSymbol);
        if Rf_isInteger(s) != 0 && LENGTH(s) == 2 && *INTEGER(s) == i32::MIN {
            let n = (*INTEGER(s).add(1)).abs();
            let r = core().alloc_vector(INTSXP, n as usize);
            let p = r.std_data() as *mut i32;
            for i in 0..n as usize {
                *p.add(i) = i as i32 + 1;
            }
            return r;
        }
        return s;
    }
    get_attrib0(vec, name)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetOption1(tag: Sexp) -> Sexp {
    crate::host::op("getOption\0", &[tag])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetOption(tag: Sexp, _rho: Sexp) -> Sexp {
    Rf_GetOption1(tag)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetOptionWidth() -> c_int {
    let w = Rf_GetOption1(sym("width"));
    crate::coerce::Rf_asInteger(w)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetOptionDigits() -> c_int {
    let w = Rf_GetOption1(sym("digits"));
    crate::coerce::Rf_asInteger(w)
}

unsafe fn strip_attrib(tag: Sexp, lst: Sexp) -> Sexp {
    if lst == R_NilValue {
        return lst;
    }
    if tag == lst.tag() {
        return strip_attrib(tag, lst.cdr());
    }
    SETCDR(lst, strip_attrib(tag, lst.cdr()));
    lst
}

// R: attrib.c:installAttrib
pub unsafe fn install_attrib(vec: Sexp, name: Sexp, val: Sexp) -> Sexp {
    match vec.typ() {
        CHARSXP => error_str("cannot set attribute on a CHARSXP"),
        SYMSXP => error_str_owned(format!("cannot set attribute on a '{}'", type_to_char(vec))),
        _ => {}
    }
    let mut t = R_NilValue;
    let mut s = ATTRIB(vec);
    while s != R_NilValue {
        if s.tag() == name {
            let val = if MAYBE_REFERENCED(val) != 0 && val != s.car() && val == vec { Rf_duplicate(val) } else { val };
            SETCAR(s, val);
            return val;
        }
        t = s;
        s = s.cdr();
    }
    Rf_protect(vec);
    Rf_protect(name);
    Rf_protect(val);
    if MAYBE_REFERENCED(val) != 0 {
        ENSURE_NAMEDMAX(val);
    }
    let s = crate::memory::Rf_cons(val, R_NilValue);
    SET_TAG(s, name);
    if ATTRIB(vec) == R_NilValue {
        SET_ATTRIB(vec, s);
    } else {
        SETCDR(t, s);
    }
    Rf_unprotect(3);
    val
}

// R: attrib.c:removeAttrib
unsafe fn remove_attrib(vec: Sexp, name: Sexp) -> Sexp {
    if vec.typ() == CHARSXP {
        error_str("cannot set attribute on a CHARSXP")
    }
    if name == R_NamesSymbol && Rf_isPairList(vec) != 0 {
        let mut t = vec;
        while t != R_NilValue {
            SET_TAG(t, R_NilValue);
            t = t.cdr();
        }
        return R_NilValue;
    }
    if name == R_DimSymbol {
        SET_ATTRIB(vec, strip_attrib(R_DimNamesSymbol, ATTRIB(vec)));
    }
    SET_ATTRIB(vec, strip_attrib(name, ATTRIB(vec)));
    if name == R_ClassSymbol {
        SET_OBJECT(vec, 0);
    }
    R_NilValue
}

// R: attrib.c:setAttrib
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_setAttrib(vec: Sexp, name: Sexp, val: Sexp) -> Sexp {
    Rf_protect(vec);
    Rf_protect(name);
    let name = if name.typ() == STRSXP {
        Rf_protect(val);
        let n = Rf_installTrChar(STRING_ELT(name, 0));
        Rf_unprotect(1);
        n
    } else {
        name
    };
    if val == R_NilValue {
        Rf_unprotect(2);
        return remove_attrib(vec, name);
    }
    if vec == R_NilValue {
        error_str("attempt to set an attribute on NULL")
    }
    Rf_unprotect(2);
    if name == R_NamesSymbol {
        Rf_namesgets(vec, val)
    } else if name == R_DimSymbol {
        Rf_dimgets(vec, val)
    } else if name == R_DimNamesSymbol {
        Rf_dimnamesgets(vec, val)
    } else if name == R_ClassSymbol {
        Rf_classgets(vec, val)
    } else if name == R_TspSymbol {
        Rf_tspgets(vec, val)
    } else if name == sym("comment") {
        commentgets(vec, val)
    } else if name == R_RowNamesSymbol {
        row_names_gets(vec, val)
    } else {
        install_attrib(vec, name, val)
    }
}

// R: attrib.c:row_names_gets
unsafe fn row_names_gets(vec: Sexp, val: Sexp) -> Sexp {
    if vec == R_NilValue {
        error_str("attempt to set an attribute on NULL")
    }
    if Rf_isReal(val) != 0 && LENGTH(val) == 2 && (*REAL(val)).is_nan() {
        Rf_protect(vec);
        let v = Rf_protect(crate::coerce::Rf_coerceVector(val, INTSXP as u32));
        let ans = install_attrib(vec, R_RowNamesSymbol, v);
        Rf_unprotect(2);
        return ans;
    }
    if Rf_isInteger(val) != 0 {
        let mut ok = true;
        let mut n = LENGTH(val);
        let p = INTEGER(val);
        if n == 2 && *p == i32::MIN {
            n = *p.add(1);
        } else if n > 2 {
            for i in 0..n as usize {
                if *p.add(i) != i as i32 + 1 {
                    ok = false;
                    break;
                }
            }
        } else {
            ok = false;
        }
        if ok {
            Rf_protect(vec);
            let v = Rf_protect(core().alloc_vector(INTSXP, 2));
            *INTEGER(v) = i32::MIN;
            *INTEGER(v).add(1) = n;
            let ans = install_attrib(vec, R_RowNamesSymbol, v);
            Rf_unprotect(2);
            return ans;
        }
    } else if Rf_isString(val) == 0 {
        error_str_owned(format!("row names must be 'character' or 'integer', not '{}'", type_to_char(val)))
    }
    install_attrib(vec, R_RowNamesSymbol, val)
}

// R: attrib.c:commentgets
unsafe fn commentgets(vec: Sexp, comment: Sexp) -> Sexp {
    if vec == R_NilValue {
        error_str("attempt to set an attribute on NULL")
    }
    if Rf_isNull(comment) != 0 || Rf_isString(comment) != 0 {
        if Rf_length(comment) <= 0 {
            SET_ATTRIB(vec, strip_attrib(sym("comment"), ATTRIB(vec)));
        } else {
            install_attrib(vec, sym("comment"), comment);
        }
        return R_NilValue;
    }
    error_str("attempt to set invalid 'comment' attribute")
}

// R: attrib.c:classgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_classgets(vec: Sexp, klass: Sexp) -> Sexp {
    if Rf_isNull(klass) != 0 || Rf_isString(klass) != 0 {
        let ncl = Rf_length(klass);
        if ncl <= 0 {
            SET_ATTRIB(vec, strip_attrib(R_ClassSymbol, ATTRIB(vec)));
            SET_OBJECT(vec, 0);
        } else {
            if vec == R_NilValue {
                error_str("attempt to set an attribute on NULL")
            }
            let isfactor = (0..ncl as isize).any(|i| STRING_ELT(klass, i).char_bytes() == b"factor");
            if isfactor && vec.typ() != INTSXP {
                error_str("adding class \"factor\" to an invalid object")
            }
            install_attrib(vec, R_ClassSymbol, klass);
            SET_OBJECT(vec, 1);
        }
        return R_NilValue;
    }
    error_str("attempt to set invalid 'class' attribute")
}

// R: attrib.c:tspgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_tspgets(vec: Sexp, val: Sexp) -> Sexp {
    if vec == R_NilValue {
        error_str("attempt to set an attribute on NULL")
    }
    if IS_S4_OBJECT(vec) != 0 {
        if Rf_isNumeric(val) == 0 {
            error_str("'tsp' attribute must be numeric")
        }
        install_attrib(vec, R_TspSymbol, val);
        return vec;
    }
    if Rf_isNumeric(val) == 0 || LENGTH(val) != 3 {
        error_str("'tsp' attribute must be numeric of length three")
    }
    let get = |i: usize| -> f64 {
        if Rf_isReal(val) != 0 {
            *REAL(val).add(i)
        } else {
            let v = *INTEGER(val).add(i);
            if v == i32::MIN { robigo_core::NA_REAL } else { v as f64 }
        }
    };
    let (start, end, freq) = (get(0), get(1), get(2));
    if freq <= 0.0 {
        error_str("invalid time series parameters specified")
    }
    let n = Rf_nrows(vec);
    if n == 0 {
        error_str("cannot assign 'tsp' to zero-length vector")
    }
    if (end - start - (n - 1) as f64 / freq).abs() > 1e-5 {
        error_str("invalid time series parameters specified")
    }
    Rf_protect(vec);
    let v = Rf_protect(core().alloc_vector(REALSXP, 3));
    *REAL(v) = start;
    *REAL(v).add(1) = end;
    *REAL(v).add(2) = freq;
    install_attrib(vec, R_TspSymbol, v);
    Rf_unprotect(2);
    vec
}

unsafe fn check_names(x: Sexp, s: Sexp) {
    if is_vector(x) || Rf_isList(x) != 0 || Rf_isLanguage(x) != 0 {
        if !is_vector(s) && Rf_isList(s) == 0 {
            error_str_owned(format!("invalid type ({}) for 'names': must be vector or NULL", type_to_char(s)))
        }
        if Rf_xlength(x) != Rf_xlength(s) {
            error_str_owned(format!(
                "'names' attribute [{}] must be the same length as the vector [{}]",
                Rf_xlength(s),
                Rf_xlength(x)
            ))
        }
    } else if IS_S4_OBJECT(x) != 0 {
    } else {
        error_str("names() applied to a non-vector")
    }
}

// R: attrib.c:namesgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_namesgets(vec: Sexp, val: Sexp) -> Sexp {
    Rf_protect(vec);
    Rf_protect(val);
    let mut val = if Rf_isList(val) != 0 {
        let n = Rf_length(vec);
        let rval = Rf_protect(core().alloc_vector(STRSXP, n as usize));
        let mut tval = val;
        let mut i = 0;
        while i < n && tval != R_NilValue {
            let s = crate::coerce::Rf_coerceVector(tval.car(), STRSXP as u32);
            SET_STRING_ELT(rval, i as isize, STRING_ELT(s, 0));
            i += 1;
            tval = tval.cdr();
        }
        Rf_unprotect(1);
        rval
    } else {
        crate::coerce::Rf_coerceVector(val, STRSXP as u32)
    };
    Rf_unprotect(1);
    Rf_protect(val);
    if Rf_xlength(val) < Rf_xlength(vec) {
        val = crate::list::Rf_xlengthgets(val, Rf_xlength(vec));
        Rf_unprotect(1);
        Rf_protect(val);
    }
    check_names(vec, val);
    if is_one_dimensional_array(vec) {
        let v = Rf_protect(crate::memory::Rf_cons(val, R_NilValue));
        Rf_setAttrib(vec, R_DimNamesSymbol, v);
        Rf_unprotect(3);
        return vec;
    }
    if Rf_isList(vec) != 0 || Rf_isLanguage(vec) != 0 {
        let mut s = vec;
        let mut i = 0;
        while s != R_NilValue {
            let e = STRING_ELT(val, i);
            if e != R_NilValue && e != R_NaString && !e.char_bytes().is_empty() {
                SET_TAG(s, Rf_installTrChar(e));
            } else {
                SET_TAG(s, R_NilValue);
            }
            s = s.cdr();
            i += 1;
        }
    } else if is_vector(vec) || IS_S4_OBJECT(vec) != 0 {
        install_attrib(vec, R_NamesSymbol, val);
    } else {
        error_str_owned(format!("invalid type ({}) to set 'names' attribute", type_to_char(vec)))
    }
    Rf_unprotect(2);
    vec
}

// R: attrib.c:as_char_simpl
unsafe fn as_char_simpl(val1: Sexp) -> Sexp {
    if LENGTH(val1) == 0 {
        return R_NilValue;
    }
    if Rf_isFactor(val1) != 0 {
        return crate::coerce::Rf_asCharacterFactor(val1);
    }
    if Rf_isString(val1) == 0 {
        let t = Rf_protect(crate::coerce::Rf_coerceVector(val1, STRSXP as u32));
        SET_ATTRIB(t, R_NilValue);
        SET_OBJECT(t, 0);
        Rf_unprotect(1);
        return t;
    }
    val1
}

// R: attrib.c:dimnamesgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_dimnamesgets(vec: Sexp, val: Sexp) -> Sexp {
    Rf_protect(vec);
    Rf_protect(val);
    if Rf_isArray(vec) == 0 && Rf_isList(vec) == 0 {
        error_str("'dimnames' applied to non-array")
    }
    if Rf_isList(val) == 0 && Rf_isNewList(val) == 0 {
        error_str("'dimnames' must be a list")
    }
    let dims = Rf_getAttrib(vec, R_DimSymbol);
    let k = LENGTH(dims);
    if k < Rf_length(val) {
        error_str_owned(format!("length of 'dimnames' [{}] must match that of 'dims' [{}]", Rf_length(val), k))
    }
    if Rf_length(val) == 0 {
        remove_attrib(vec, R_DimNamesSymbol);
        Rf_unprotect(2);
        return vec;
    }
    let mut val = val;
    if Rf_isList(val) != 0 {
        let nv = core().alloc_vector(VECSXP, k as usize);
        let mut v = val;
        for i in 0..k as isize {
            SET_VECTOR_ELT(nv, i, v.car());
            v = v.cdr();
        }
        Rf_unprotect(1);
        val = Rf_protect(nv);
    }
    if Rf_length(val) > 0 && Rf_length(val) < k {
        let nv = crate::list::Rf_lengthgets(val, k);
        Rf_unprotect(1);
        val = Rf_protect(nv);
    }
    if MAYBE_REFERENCED(val) != 0 {
        let nv = Rf_shallow_duplicate(val);
        Rf_unprotect(1);
        val = Rf_protect(nv);
    }
    if k != Rf_length(val) {
        error_str_owned(format!("length of 'dimnames' [{}] must match that of 'dims' [{}]", Rf_length(val), k))
    }
    for i in 0..k as isize {
        let this = VECTOR_ELT(val, i);
        if this != R_NilValue {
            if !is_vector(this) {
                error_str_owned(format!("invalid type ({}) for 'dimnames' (must be a vector)", type_to_char(this)))
            }
            if *INTEGER(dims).offset(i) != LENGTH(this) && LENGTH(this) != 0 {
                error_str_owned(format!("length of 'dimnames' [{}] not equal to array extent", i + 1))
            }
            SET_VECTOR_ELT(val, i, as_char_simpl(this));
        }
    }
    install_attrib(vec, R_DimNamesSymbol, val);
    if Rf_isList(vec) != 0 && k == 1 {
        let top = VECTOR_ELT(val, 0);
        let mut v = vec;
        let mut i = 0;
        while v != R_NilValue {
            SET_TAG(v, Rf_installTrChar(STRING_ELT(top, i)));
            i += 1;
            v = v.cdr();
        }
    }
    Rf_unprotect(2);
    MARK_NOT_MUTABLE(val);
    vec
}

// R: attrib.c:dimgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_dimgets(vec: Sexp, val: Sexp) -> Sexp {
    Rf_protect(vec);
    Rf_protect(val);
    if !is_vector(vec) && Rf_isList(vec) == 0 {
        error_str("invalid first argument, must be vector (list or atomic)")
    }
    if val != R_NilValue && Rf_isVectorAtomic(val) == 0 {
        error_str("invalid second argument, must be vector or NULL")
    }
    let val = crate::coerce::Rf_coerceVector(val, INTSXP as u32);
    Rf_unprotect(1);
    Rf_protect(val);
    let ndim = Rf_length(val);
    if ndim == 0 {
        error_str("length-0 dimension vector is invalid")
    }
    let len = Rf_xlength(vec);
    let mut total: isize = 1;
    for i in 0..ndim as usize {
        let d = *INTEGER(val).add(i);
        if d == i32::MIN {
            error_str("the dims contain missing values")
        }
        if d < 0 {
            error_str("the dims contain negative values")
        }
        total *= d as isize;
    }
    if total != len {
        error_str_owned(format!("dims [product {total}] do not match the length of object [{len}]"))
    }
    remove_attrib(vec, R_DimNamesSymbol);
    install_attrib(vec, R_DimSymbol, val);
    MARK_NOT_MUTABLE(val);
    Rf_unprotect(2);
    vec
}

// R: attrib.c:copyMostAttrib
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_copyMostAttrib(inp: Sexp, ans: Sexp) {
    if ans == R_NilValue {
        error_str("attempt to set an attribute on NULL")
    }
    Rf_protect(ans);
    Rf_protect(inp);
    let mut s = ATTRIB(inp);
    while s != R_NilValue {
        let t = s.tag();
        if t != R_NamesSymbol && t != R_DimSymbol && t != R_DimNamesSymbol {
            install_attrib(ans, t, s.car());
        }
        s = s.cdr();
    }
    if inp.has_flag(flags::OBJECT) {
        SET_OBJECT(ans, 1);
    }
    if IS_S4_OBJECT(inp) != 0 {
        SET_S4_OBJECT(ans)
    } else {
        UNSET_S4_OBJECT(ans)
    }
    Rf_unprotect(2);
}

// R: attrib.c:copyMostAttribNoTs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_copyMostAttribNoTs(inp: Sexp, ans: Sexp) {
    if ans == R_NilValue {
        error_str("attempt to set an attribute on NULL")
    }
    let mut is_object = OBJECT(inp);
    let mut is_s4 = IS_S4_OBJECT(inp);
    Rf_protect(ans);
    Rf_protect(inp);
    let mut s = ATTRIB(inp);
    while s != R_NilValue {
        let t = s.tag();
        if t != R_NamesSymbol && t != R_ClassSymbol && t != R_TspSymbol && t != R_DimSymbol && t != R_DimNamesSymbol {
            install_attrib(ans, t, s.car());
        } else if t == R_ClassSymbol {
            let cl = s.car();
            let l = LENGTH(cl) as isize;
            let ists = (0..l).any(|i| STRING_ELT(cl, i).char_bytes() == b"ts");
            if !ists {
                install_attrib(ans, t, cl);
            } else if l <= 1 {
                is_object = 0;
                is_s4 = 0;
            } else {
                let ncl = Rf_protect(core().alloc_vector(STRSXP, l as usize - 1));
                let mut j = 0;
                for i in 0..l {
                    if STRING_ELT(cl, i).char_bytes() != b"ts" {
                        SET_STRING_ELT(ncl, j, STRING_ELT(cl, i));
                        j += 1;
                    }
                }
                install_attrib(ans, t, ncl);
                Rf_unprotect(1);
            }
        }
        s = s.cdr();
    }
    SET_OBJECT(ans, is_object);
    if is_s4 != 0 {
        SET_S4_OBJECT(ans)
    } else {
        UNSET_S4_OBJECT(ans)
    }
    Rf_unprotect(2);
}

// R: attrib.c:GetMatrixDimnames
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetMatrixDimnames(
    x: Sexp,
    rl: *mut Sexp,
    cl: *mut Sexp,
    rn: *mut *const c_char,
    cn: *mut *const c_char,
) {
    let dimnames = Rf_getAttrib(x, R_DimNamesSymbol);
    if dimnames == R_NilValue {
        *rl = R_NilValue;
        *cl = R_NilValue;
        *rn = std::ptr::null();
        *cn = std::ptr::null();
    } else {
        *rl = VECTOR_ELT(dimnames, 0);
        *cl = VECTOR_ELT(dimnames, 1);
        let nn = Rf_getAttrib(dimnames, R_NamesSymbol);
        if nn == R_NilValue {
            *rn = std::ptr::null();
            *cn = std::ptr::null();
        } else {
            *rn = crate::strings::Rf_translateChar(STRING_ELT(nn, 0));
            *cn = crate::strings::Rf_translateChar(STRING_ELT(nn, 1));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetArrayDimnames(x: Sexp) -> Sexp {
    Rf_getAttrib(x, R_DimNamesSymbol)
}

// R: util.c:GetRowNames / GetColNames (dimnames components)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetRowNames(dimnames: Sexp) -> Sexp {
    if dimnames.typ() == VECSXP { VECTOR_ELT(dimnames, 0) } else { R_NilValue }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetColNames(dimnames: Sexp) -> Sexp {
    if dimnames.typ() == VECSXP { VECTOR_ELT(dimnames, 1) } else { R_NilValue }
}

#[allow(dead_code)]
unsafe fn cstr(p: *const c_char) -> &'static [u8] {
    CStr::from_ptr(p).to_bytes()
}
