//! attrib.c: attribute access with GNU R's validation.

use crate::errors::{Res, error_str, ret};
use crate::memory::*;
use crate::state::*;
use crate::strings::install_tr_char;
use crate::util::*;
use robigo_core::types::*;
use robigo_core::{Sexp, flags};
use std::ffi::{c_char, c_int};

unsafe fn sym(name: &str) -> Sexp {
    core().install(name.as_bytes())
}

unsafe fn is_one_dimensional_array(vec: Sexp) -> Res<bool> {
    if is_vector(vec) || Rf_isList(vec) != 0 || Rf_isLanguage(vec) != 0 {
        let s = get_attrib(vec, R_DimSymbol)?;
        if s.typ() == INTSXP && length(s)? == 1 {
            return Ok(true);
        }
    }
    Ok(false)
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
pub unsafe fn get_attrib0(vec: Sexp, name: Sexp) -> Res<Sexp> {
    if name == R_NamesSymbol {
        if is_one_dimensional_array(vec)? {
            let s = get_attrib(vec, R_DimNamesSymbol)?;
            if s != R_NilValue {
                MARK_NOT_MUTABLE(vector_elt(s, 0)?);
                return vector_elt(s, 0);
            }
        }
        if matches!(vec.typ(), LISTSXP | LANGSXP | DOTSXP | NILSXP) {
            let len = r_length(vec)?;
            let s = protect(crate::memory::try_alloc_vector(STRSXP, len as usize)?)?;
            let mut any = false;
            let mut v = vec;
            let mut i = 0;
            while v != R_NilValue {
                let t = v.tag();
                if t == R_NilValue {
                    set_string_elt(s, i, R_BlankString)?;
                } else if t.typ() == SYMSXP {
                    any = true;
                    set_string_elt(s, i, t.car())?;
                } else {
                    return Err(error_str(&format!("getAttrib: invalid type ({}) for TAG", type_to_char(t))));
                }
                v = v.cdr();
                i += 1;
            }
            unprotect(1)?;
            if any {
                MARK_NOT_MUTABLE(s);
                return Ok(s);
            }
            return Ok(R_NilValue);
        }
    }
    let mut s = ATTRIB(vec);
    while s != R_NilValue {
        if s.tag() == name {
            if name == R_DimNamesSymbol && s.car().typ() == LISTSXP {
                return Err(error_str("old list is no longer allowed for dimnames attribute"));
            }
            MARK_NOT_MUTABLE(s.car());
            return Ok(s.car());
        }
        s = s.cdr();
    }
    Ok(R_NilValue)
}

// R: attrib.c:getAttrib
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_getAttrib(vec: Sexp, name: Sexp) -> Sexp {
    ret(get_attrib(vec, name))
}

/// getAttrib.
pub unsafe fn get_attrib(vec: Sexp, name: Sexp) -> Res<Sexp> {
    if vec.typ() == CHARSXP {
        return Err(error_str("cannot have attributes on a CHARSXP"));
    }
    if ATTRIB(vec) == R_NilValue && !matches!(vec.typ(), LISTSXP | LANGSXP | DOTSXP) {
        return Ok(R_NilValue);
    }
    let name = if name.typ() == STRSXP { install_tr_char(string_elt(name, 0)?)? } else { name };
    if name == R_RowNamesSymbol {
        let s = get_attrib0(vec, R_RowNamesSymbol)?;
        if Rf_isInteger(s) != 0 && length(s)? == 2 && *integer(s)? == i32::MIN {
            let n = (*integer(s)?.add(1)).abs();
            let r = crate::memory::try_alloc_vector(INTSXP, n as usize)?;
            let p = r.std_data() as *mut i32;
            for i in 0..n as usize {
                *p.add(i) = i as i32 + 1;
            }
            return Ok(r);
        }
        return Ok(s);
    }
    get_attrib0(vec, name)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetOption1(tag: Sexp) -> Sexp {
    ret(get_option1(tag))
}

/// GetOption1.
pub unsafe fn get_option1(tag: Sexp) -> Res<Sexp> {
    crate::host::op("getOption\0", &[tag])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetOption(tag: Sexp, _rho: Sexp) -> Sexp {
    ret(get_option1(tag))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetOptionWidth() -> c_int {
    ret(get_option1(sym("width")).and_then(|w| crate::coerce::as_integer(w)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetOptionDigits() -> c_int {
    ret(get_option1(sym("digits")).and_then(|w| crate::coerce::as_integer(w)))
}

unsafe fn strip_attrib(tag: Sexp, lst: Sexp) -> Res<Sexp> {
    if lst == R_NilValue {
        return Ok(lst);
    }
    if tag == lst.tag() {
        return strip_attrib(tag, lst.cdr());
    }
    setcdr(lst, strip_attrib(tag, lst.cdr())?)?;
    Ok(lst)
}

// R: attrib.c:installAttrib
pub unsafe fn install_attrib(vec: Sexp, name: Sexp, val: Sexp) -> Res<Sexp> {
    match vec.typ() {
        CHARSXP => return Err(error_str("cannot set attribute on a CHARSXP")),
        SYMSXP => return Err(error_str(&format!("cannot set attribute on a '{}'", type_to_char(vec)))),
        _ => {}
    }
    let mut t = R_NilValue;
    let mut s = ATTRIB(vec);
    while s != R_NilValue {
        if s.tag() == name {
            let val = if MAYBE_REFERENCED(val) != 0 && val != s.car() && val == vec { duplicate(val)? } else { val };
            setcar(s, val)?;
            return Ok(val);
        }
        t = s;
        s = s.cdr();
    }
    protect(vec)?;
    protect(name)?;
    protect(val)?;
    if MAYBE_REFERENCED(val) != 0 {
        ENSURE_NAMEDMAX(val);
    }
    let s = crate::memory::Rf_cons(val, R_NilValue);
    set_tag(s, name)?;
    if ATTRIB(vec) == R_NilValue {
        set_attrib_list(vec, s)?;
    } else {
        setcdr(t, s)?;
    }
    unprotect(3)?;
    Ok(val)
}

// R: attrib.c:removeAttrib
unsafe fn remove_attrib(vec: Sexp, name: Sexp) -> Res<Sexp> {
    if vec.typ() == CHARSXP {
        return Err(error_str("cannot set attribute on a CHARSXP"));
    }
    if name == R_NamesSymbol && Rf_isPairList(vec) != 0 {
        let mut t = vec;
        while t != R_NilValue {
            set_tag(t, R_NilValue)?;
            t = t.cdr();
        }
        return Ok(R_NilValue);
    }
    if name == R_DimSymbol {
        set_attrib_list(vec, strip_attrib(R_DimNamesSymbol, ATTRIB(vec))?)?;
    }
    set_attrib_list(vec, strip_attrib(name, ATTRIB(vec))?)?;
    if name == R_ClassSymbol {
        SET_OBJECT(vec, 0);
    }
    Ok(R_NilValue)
}

// R: attrib.c:setAttrib
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_setAttrib(vec: Sexp, name: Sexp, val: Sexp) -> Sexp {
    if crate::memory::root_args() {
        return ret(crate::memory::rooted(&[vec, name, val], || set_attrib(vec, name, val)));
    }
    ret(set_attrib(vec, name, val))
}

/// setAttrib.
pub unsafe fn set_attrib(vec: Sexp, name: Sexp, val: Sexp) -> Res<Sexp> {
    protect(vec)?;
    protect(name)?;
    let name = if name.typ() == STRSXP {
        protect(val)?;
        let n = install_tr_char(string_elt(name, 0)?)?;
        unprotect(1)?;
        n
    } else {
        name
    };
    if val == R_NilValue {
        unprotect(2)?;
        return remove_attrib(vec, name);
    }
    if vec == R_NilValue {
        return Err(error_str("attempt to set an attribute on NULL"));
    }
    unprotect(2)?;
    if name == R_NamesSymbol {
        namesgets(vec, val)
    } else if name == R_DimSymbol {
        dimgets(vec, val)
    } else if name == R_DimNamesSymbol {
        dimnamesgets(vec, val)
    } else if name == R_ClassSymbol {
        classgets(vec, val)
    } else if name == R_TspSymbol {
        tspgets(vec, val)
    } else if name == sym("comment") {
        commentgets(vec, val)
    } else if name == R_RowNamesSymbol {
        row_names_gets(vec, val)
    } else {
        install_attrib(vec, name, val)
    }
}

// R: attrib.c:row_names_gets
unsafe fn row_names_gets(vec: Sexp, val: Sexp) -> Res<Sexp> {
    if vec == R_NilValue {
        return Err(error_str("attempt to set an attribute on NULL"));
    }
    if Rf_isReal(val) != 0 && length(val)? == 2 && (*real(val)?).is_nan() {
        protect(vec)?;
        let v = protect(crate::coerce::coerce_vector(val, INTSXP as u32)?)?;
        let ans = install_attrib(vec, R_RowNamesSymbol, v)?;
        unprotect(2)?;
        return Ok(ans);
    }
    if Rf_isInteger(val) != 0 {
        let mut ok = true;
        let mut n = length(val)?;
        let p = integer(val)?;
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
            protect(vec)?;
            let v = protect(core().alloc_vector(INTSXP, 2))?;
            let d = v.std_data() as *mut i32;
            *d = i32::MIN;
            *d.add(1) = n;
            let ans = install_attrib(vec, R_RowNamesSymbol, v)?;
            unprotect(2)?;
            return Ok(ans);
        }
    } else if Rf_isString(val) == 0 {
        return Err(error_str(&format!("row names must be 'character' or 'integer', not '{}'", type_to_char(val))));
    }
    install_attrib(vec, R_RowNamesSymbol, val)
}

// R: attrib.c:commentgets
unsafe fn commentgets(vec: Sexp, comment: Sexp) -> Res<Sexp> {
    if vec == R_NilValue {
        return Err(error_str("attempt to set an attribute on NULL"));
    }
    if Rf_isNull(comment) != 0 || Rf_isString(comment) != 0 {
        if r_length(comment)? <= 0 {
            set_attrib_list(vec, strip_attrib(sym("comment"), ATTRIB(vec))?)?;
        } else {
            install_attrib(vec, sym("comment"), comment)?;
        }
        return Ok(R_NilValue);
    }
    Err(error_str("attempt to set invalid 'comment' attribute"))
}

// R: attrib.c:classgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_classgets(vec: Sexp, klass: Sexp) -> Sexp {
    ret(classgets(vec, klass))
}

/// classgets.
pub unsafe fn classgets(vec: Sexp, klass: Sexp) -> Res<Sexp> {
    if Rf_isNull(klass) != 0 || Rf_isString(klass) != 0 {
        let ncl = r_length(klass)?;
        if ncl <= 0 {
            set_attrib_list(vec, strip_attrib(R_ClassSymbol, ATTRIB(vec))?)?;
            SET_OBJECT(vec, 0);
        } else {
            if vec == R_NilValue {
                return Err(error_str("attempt to set an attribute on NULL"));
            }
            let mut isfactor = false;
            for i in 0..ncl as isize {
                if string_elt(klass, i)?.char_bytes() == b"factor" {
                    isfactor = true;
                    break;
                }
            }
            if isfactor && vec.typ() != INTSXP {
                return Err(error_str("adding class \"factor\" to an invalid object"));
            }
            install_attrib(vec, R_ClassSymbol, klass)?;
            SET_OBJECT(vec, 1);
        }
        return Ok(R_NilValue);
    }
    Err(error_str("attempt to set invalid 'class' attribute"))
}

// R: attrib.c:tspgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_tspgets(vec: Sexp, val: Sexp) -> Sexp {
    ret(tspgets(vec, val))
}

unsafe fn tspgets(vec: Sexp, val: Sexp) -> Res<Sexp> {
    if vec == R_NilValue {
        return Err(error_str("attempt to set an attribute on NULL"));
    }
    if IS_S4_OBJECT(vec) != 0 {
        if Rf_isNumeric(val) == 0 {
            return Err(error_str("'tsp' attribute must be numeric"));
        }
        install_attrib(vec, R_TspSymbol, val)?;
        return Ok(vec);
    }
    if Rf_isNumeric(val) == 0 || length(val)? != 3 {
        return Err(error_str("'tsp' attribute must be numeric of length three"));
    }
    let get = |i: usize| -> Res<f64> {
        Ok(if Rf_isReal(val) != 0 {
            *real(val)?.add(i)
        } else {
            let v = *integer(val)?.add(i);
            if v == i32::MIN { robigo_core::NA_REAL } else { v as f64 }
        })
    };
    let (start, end, freq) = (get(0)?, get(1)?, get(2)?);
    if freq <= 0.0 {
        return Err(error_str("invalid time series parameters specified"));
    }
    let n = nrows(vec)?;
    if n == 0 {
        return Err(error_str("cannot assign 'tsp' to zero-length vector"));
    }
    if (end - start - (n - 1) as f64 / freq).abs() > 1e-5 {
        return Err(error_str("invalid time series parameters specified"));
    }
    protect(vec)?;
    let v = protect(core().alloc_vector(REALSXP, 3))?;
    let d = v.std_data() as *mut f64;
    *d = start;
    *d.add(1) = end;
    *d.add(2) = freq;
    install_attrib(vec, R_TspSymbol, v)?;
    unprotect(2)?;
    Ok(vec)
}

unsafe fn check_names(x: Sexp, s: Sexp) -> Res<()> {
    if is_vector(x) || Rf_isList(x) != 0 || Rf_isLanguage(x) != 0 {
        if !is_vector(s) && Rf_isList(s) == 0 {
            return Err(error_str(&format!("invalid type ({}) for 'names': must be vector or NULL", type_to_char(s))));
        }
        if r_xlength(x)? != r_xlength(s)? {
            return Err(error_str(&format!(
                "'names' attribute [{}] must be the same length as the vector [{}]",
                r_xlength(s)?,
                r_xlength(x)?
            )));
        }
    } else if IS_S4_OBJECT(x) != 0 {
    } else {
        return Err(error_str("names() applied to a non-vector"));
    }
    Ok(())
}

// R: attrib.c:namesgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_namesgets(vec: Sexp, val: Sexp) -> Sexp {
    ret(namesgets(vec, val))
}

/// namesgets.
pub unsafe fn namesgets(vec: Sexp, val: Sexp) -> Res<Sexp> {
    protect(vec)?;
    protect(val)?;
    let mut val = if Rf_isList(val) != 0 {
        let n = r_length(vec)?;
        let rval = protect(crate::memory::try_alloc_vector(STRSXP, n as usize)?)?;
        let mut tval = val;
        let mut i = 0;
        while i < n && tval != R_NilValue {
            let s = crate::coerce::coerce_vector(tval.car(), STRSXP as u32)?;
            set_string_elt(rval, i as isize, string_elt(s, 0)?)?;
            i += 1;
            tval = tval.cdr();
        }
        unprotect(1)?;
        rval
    } else {
        crate::coerce::coerce_vector(val, STRSXP as u32)?
    };
    unprotect(1)?;
    protect(val)?;
    if r_xlength(val)? < r_xlength(vec)? {
        val = crate::list::xlengthgets(val, r_xlength(vec)?)?;
        unprotect(1)?;
        protect(val)?;
    }
    check_names(vec, val)?;
    if is_one_dimensional_array(vec)? {
        let v = protect(crate::memory::Rf_cons(val, R_NilValue))?;
        set_attrib(vec, R_DimNamesSymbol, v)?;
        unprotect(3)?;
        return Ok(vec);
    }
    if Rf_isList(vec) != 0 || Rf_isLanguage(vec) != 0 {
        let mut s = vec;
        let mut i = 0;
        while s != R_NilValue {
            let e = string_elt(val, i)?;
            if e != R_NilValue && e != R_NaString && !e.char_bytes().is_empty() {
                set_tag(s, install_tr_char(e)?)?;
            } else {
                set_tag(s, R_NilValue)?;
            }
            s = s.cdr();
            i += 1;
        }
    } else if is_vector(vec) || IS_S4_OBJECT(vec) != 0 {
        install_attrib(vec, R_NamesSymbol, val)?;
    } else {
        return Err(error_str(&format!("invalid type ({}) to set 'names' attribute", type_to_char(vec))));
    }
    unprotect(2)?;
    Ok(vec)
}

// R: attrib.c:as_char_simpl
unsafe fn as_char_simpl(val1: Sexp) -> Res<Sexp> {
    if length(val1)? == 0 {
        return Ok(R_NilValue);
    }
    if Rf_isFactor(val1) != 0 {
        return crate::coerce::as_character_factor(val1);
    }
    if Rf_isString(val1) == 0 {
        let t = protect(crate::coerce::coerce_vector(val1, STRSXP as u32)?)?;
        set_attrib_list(t, R_NilValue)?;
        SET_OBJECT(t, 0);
        unprotect(1)?;
        return Ok(t);
    }
    Ok(val1)
}

// R: attrib.c:dimnamesgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_dimnamesgets(vec: Sexp, val: Sexp) -> Sexp {
    ret(dimnamesgets(vec, val))
}

/// dimnamesgets.
pub unsafe fn dimnamesgets(vec: Sexp, val: Sexp) -> Res<Sexp> {
    protect(vec)?;
    protect(val)?;
    if Rf_isArray(vec) == 0 && Rf_isList(vec) == 0 {
        return Err(error_str("'dimnames' applied to non-array"));
    }
    if Rf_isList(val) == 0 && Rf_isNewList(val) == 0 {
        return Err(error_str("'dimnames' must be a list"));
    }
    let dims = get_attrib(vec, R_DimSymbol)?;
    let k = length(dims)?;
    if k < r_length(val)? {
        return Err(error_str(&format!("length of 'dimnames' [{}] must match that of 'dims' [{}]", r_length(val)?, k)));
    }
    if r_length(val)? == 0 {
        remove_attrib(vec, R_DimNamesSymbol)?;
        unprotect(2)?;
        return Ok(vec);
    }
    let mut val = val;
    if Rf_isList(val) != 0 {
        let nv = crate::memory::try_alloc_vector(VECSXP, k as usize)?;
        let mut v = val;
        for i in 0..k as isize {
            set_vector_elt(nv, i, v.car())?;
            v = v.cdr();
        }
        unprotect(1)?;
        val = protect(nv)?;
    }
    if r_length(val)? > 0 && r_length(val)? < k {
        let nv = crate::list::lengthgets(val, k)?;
        unprotect(1)?;
        val = protect(nv)?;
    }
    if MAYBE_REFERENCED(val) != 0 {
        let nv = shallow_duplicate(val)?;
        unprotect(1)?;
        val = protect(nv)?;
    }
    if k != r_length(val)? {
        return Err(error_str(&format!("length of 'dimnames' [{}] must match that of 'dims' [{}]", r_length(val)?, k)));
    }
    for i in 0..k as isize {
        let this = vector_elt(val, i)?;
        if this != R_NilValue {
            if !is_vector(this) {
                return Err(error_str(&format!(
                    "invalid type ({}) for 'dimnames' (must be a vector)",
                    type_to_char(this)
                )));
            }
            if *integer(dims)?.offset(i) != length(this)? && length(this)? != 0 {
                return Err(error_str(&format!("length of 'dimnames' [{}] not equal to array extent", i + 1)));
            }
            set_vector_elt(val, i, as_char_simpl(this)?)?;
        }
    }
    install_attrib(vec, R_DimNamesSymbol, val)?;
    if Rf_isList(vec) != 0 && k == 1 {
        let top = vector_elt(val, 0)?;
        let mut v = vec;
        let mut i = 0;
        while v != R_NilValue {
            set_tag(v, install_tr_char(string_elt(top, i)?)?)?;
            i += 1;
            v = v.cdr();
        }
    }
    unprotect(2)?;
    MARK_NOT_MUTABLE(val);
    Ok(vec)
}

// R: attrib.c:dimgets
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_dimgets(vec: Sexp, val: Sexp) -> Sexp {
    ret(dimgets(vec, val))
}

/// dimgets.
pub unsafe fn dimgets(vec: Sexp, val: Sexp) -> Res<Sexp> {
    protect(vec)?;
    protect(val)?;
    if !is_vector(vec) && Rf_isList(vec) == 0 {
        return Err(error_str("invalid first argument, must be vector (list or atomic)"));
    }
    if val != R_NilValue && Rf_isVectorAtomic(val) == 0 {
        return Err(error_str("invalid second argument, must be vector or NULL"));
    }
    let val = crate::coerce::coerce_vector(val, INTSXP as u32)?;
    unprotect(1)?;
    protect(val)?;
    let ndim = r_length(val)?;
    if ndim == 0 {
        return Err(error_str("length-0 dimension vector is invalid"));
    }
    let len = r_xlength(vec)?;
    let mut total: isize = 1;
    for i in 0..ndim as usize {
        let d = *integer(val)?.add(i);
        if d == i32::MIN {
            return Err(error_str("the dims contain missing values"));
        }
        if d < 0 {
            return Err(error_str("the dims contain negative values"));
        }
        total *= d as isize;
    }
    if total != len {
        return Err(error_str(&format!("dims [product {total}] do not match the length of object [{len}]")));
    }
    remove_attrib(vec, R_DimNamesSymbol)?;
    install_attrib(vec, R_DimSymbol, val)?;
    MARK_NOT_MUTABLE(val);
    unprotect(2)?;
    Ok(vec)
}

// R: attrib.c:copyMostAttrib
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_copyMostAttrib(inp: Sexp, ans: Sexp) {
    ret(copy_most_attrib(inp, ans))
}

/// copyMostAttrib.
pub unsafe fn copy_most_attrib(inp: Sexp, ans: Sexp) -> Res<()> {
    if ans == R_NilValue {
        return Err(error_str("attempt to set an attribute on NULL"));
    }
    protect(ans)?;
    protect(inp)?;
    let mut s = ATTRIB(inp);
    while s != R_NilValue {
        let t = s.tag();
        if t != R_NamesSymbol && t != R_DimSymbol && t != R_DimNamesSymbol {
            install_attrib(ans, t, s.car())?;
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
    unprotect(2)
}

// R: attrib.c:copyMostAttribNoTs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_copyMostAttribNoTs(inp: Sexp, ans: Sexp) {
    ret(copy_most_attrib_no_ts(inp, ans))
}

unsafe fn copy_most_attrib_no_ts(inp: Sexp, ans: Sexp) -> Res<()> {
    if ans == R_NilValue {
        return Err(error_str("attempt to set an attribute on NULL"));
    }
    let mut is_object = OBJECT(inp);
    let mut is_s4 = IS_S4_OBJECT(inp);
    protect(ans)?;
    protect(inp)?;
    let mut s = ATTRIB(inp);
    while s != R_NilValue {
        let t = s.tag();
        if t != R_NamesSymbol && t != R_ClassSymbol && t != R_TspSymbol && t != R_DimSymbol && t != R_DimNamesSymbol {
            install_attrib(ans, t, s.car())?;
        } else if t == R_ClassSymbol {
            let cl = s.car();
            let l = length(cl)? as isize;
            let mut ists = false;
            for i in 0..l {
                if string_elt(cl, i)?.char_bytes() == b"ts" {
                    ists = true;
                    break;
                }
            }
            if !ists {
                install_attrib(ans, t, cl)?;
            } else if l <= 1 {
                is_object = 0;
                is_s4 = 0;
            } else {
                let ncl = protect(crate::memory::try_alloc_vector(STRSXP, l as usize - 1)?)?;
                let mut j = 0;
                for i in 0..l {
                    if string_elt(cl, i)?.char_bytes() != b"ts" {
                        set_string_elt(ncl, j, string_elt(cl, i)?)?;
                        j += 1;
                    }
                }
                install_attrib(ans, t, ncl)?;
                unprotect(1)?;
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
    unprotect(2)
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
    ret(get_matrix_dimnames(x, rl, cl, rn, cn))
}

unsafe fn get_matrix_dimnames(
    x: Sexp,
    rl: *mut Sexp,
    cl: *mut Sexp,
    rn: *mut *const c_char,
    cn: *mut *const c_char,
) -> Res<()> {
    let dimnames = get_attrib(x, R_DimNamesSymbol)?;
    if dimnames == R_NilValue {
        *rl = R_NilValue;
        *cl = R_NilValue;
        *rn = std::ptr::null();
        *cn = std::ptr::null();
    } else {
        *rl = vector_elt(dimnames, 0)?;
        *cl = vector_elt(dimnames, 1)?;
        let nn = get_attrib(dimnames, R_NamesSymbol)?;
        if nn == R_NilValue {
            *rn = std::ptr::null();
            *cn = std::ptr::null();
        } else {
            *rn = crate::strings::translate_char(string_elt(nn, 0)?)?;
            *cn = crate::strings::translate_char(string_elt(nn, 1)?)?;
        }
    }
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetArrayDimnames(x: Sexp) -> Sexp {
    ret(get_attrib(x, R_DimNamesSymbol))
}

// R: util.c:GetRowNames / GetColNames (dimnames components)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetRowNames(dimnames: Sexp) -> Sexp {
    if dimnames.typ() == VECSXP { ret(vector_elt(dimnames, 0)) } else { R_NilValue }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_GetColNames(dimnames: Sexp) -> Sexp {
    if dimnames.typ() == VECSXP { ret(vector_elt(dimnames, 1)) } else { R_NilValue }
}
