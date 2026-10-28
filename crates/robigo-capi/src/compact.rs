//! GNU R's compact sequences (altclasses.c): the ALTREP classes
//! `compact_intseq` and `compact_realseq` of package base. The runtime
//! marks the vectors `:`, `seq_len` and `seq_along` make (`Vector::compact`);
//! the bridge hands them to C as these objects, so `ALTREP(x)` is TRUE
//! and `INTEGER_OR_NULL(x)` is NULL until C asks for the data.
//!
//! data1 is the info `c(length, first, increment)` (a REALSXP, so long
//! vectors fit), data2 the expanded vector once DATAPTR was taken.

use std::ffi::{c_int, c_void};

use robigo_core::Sexp;
use robigo_core::types::*;

use crate::altrep::ClassT;
use crate::memory::*;
use crate::state::*;

const SORTED_DECR: c_int = -1;
const SORTED_INCR: c_int = 1;

static mut INTSEQ: Option<ClassT> = None;
static mut REALSEQ: Option<ClassT> = None;

unsafe fn info(x: Sexp) -> (isize, f64, f64) {
    let i = std::slice::from_raw_parts(x.car().std_data() as *const f64, 3);
    (i[0] as isize, i[1], i[2])
}

unsafe fn expanded(x: Sexp) -> Sexp {
    x.cdr()
}

// ---- compact_intseq ------------------------------------------------------

unsafe extern "C" fn intseq_length(x: Sexp) -> isize {
    info(x).0
}

unsafe extern "C" fn intseq_elt(x: Sexp, i: isize) -> c_int {
    let e = expanded(x);
    if e != R_NilValue {
        return *(e.std_data() as *const c_int).offset(i);
    }
    let (_, n1, inc) = info(x);
    (n1 as c_int).wrapping_add((inc as c_int).wrapping_mul(i as c_int))
}

unsafe extern "C" fn intseq_dataptr(x: Sexp, _writable: c_int) -> *mut c_void {
    if expanded(x) == R_NilValue {
        let (n, n1, inc) = info(x);
        let v = Rf_protect(alloc_vector(INTSXP as u32, n));
        let d = v.std_data() as *mut c_int;
        for i in 0..n {
            *d.offset(i) = n1 as c_int + inc as c_int * i as c_int;
        }
        crate::altrep::R_set_altrep_data2(x, v);
        Rf_unprotect(1);
    }
    expanded(x).std_data() as *mut c_void
}

unsafe extern "C" fn intseq_dataptr_or_null(x: Sexp) -> *const c_void {
    let e = expanded(x);
    if e == R_NilValue { std::ptr::null() } else { e.std_data() as *const c_void }
}

unsafe extern "C" fn intseq_get_region(x: Sexp, i: isize, n: isize, buf: *mut c_int) -> isize {
    let size = info(x).0;
    let ncopy = if size - i > n { n } else { size - i };
    for k in 0..ncopy.max(0) {
        *buf.offset(k) = intseq_elt(x, i + k);
    }
    ncopy
}

unsafe extern "C" fn intseq_is_sorted(x: Sexp) -> c_int {
    if info(x).2 < 0.0 { SORTED_DECR } else { SORTED_INCR }
}

unsafe extern "C" fn seq_no_na(_: Sexp) -> c_int {
    1
}

// R: altclasses.c:compact_intseq_Sum
unsafe extern "C" fn intseq_sum(x: Sexp, _narm: c_int) -> Sexp {
    let (size, n1, inc) = info(x);
    let tmp = (size as f64 / 2.0) * (n1 + n1 + inc * (size - 1) as f64);
    if tmp > c_int::MAX as f64 || tmp < (c_int::MIN + 1) as f64 {
        crate::list::Rf_ScalarReal(tmp)
    } else {
        crate::list::Rf_ScalarInteger(tmp as c_int)
    }
}

unsafe extern "C" fn intseq_duplicate(x: Sexp, _deep: c_int) -> Sexp {
    let n = info(x).0;
    let v = Rf_protect(alloc_vector(INTSXP as u32, n));
    intseq_get_region(x, 0, n, v.std_data() as *mut c_int);
    Rf_unprotect(1);
    v
}

unsafe extern "C" fn intseq_coerce(x: Sexp, t: c_int) -> Sexp {
    if t == REALSXP as c_int {
        let (n, n1, inc) = info(x);
        new_compact_realseq(n, n1, inc)
    } else {
        Sexp::NULL
    }
}

unsafe extern "C" fn seq_serialized_state(x: Sexp) -> Sexp {
    x.car()
}

unsafe extern "C" fn intseq_unserialize(_class: Sexp, state: Sexp) -> Sexp {
    let i = std::slice::from_raw_parts(state.std_data() as *const f64, 3);
    if i[2] != 1.0 && i[2] != -1.0 {
        crate::errors::error_str_owned(format!("compact sequences with increment {} not supported yet", i[2] as c_int))
    }
    new_compact_intseq(i[0] as isize, i[1] as c_int, i[2] as c_int)
}

// ---- compact_realseq -----------------------------------------------------

unsafe extern "C" fn realseq_elt(x: Sexp, i: isize) -> f64 {
    let e = expanded(x);
    if e != R_NilValue {
        return *(e.std_data() as *const f64).offset(i);
    }
    let (_, n1, inc) = info(x);
    n1 + inc * i as f64
}

unsafe extern "C" fn realseq_dataptr(x: Sexp, _writable: c_int) -> *mut c_void {
    if expanded(x) == R_NilValue {
        let (n, n1, inc) = info(x);
        let v = Rf_protect(alloc_vector(REALSXP as u32, n));
        let d = v.std_data() as *mut f64;
        for i in 0..n {
            *d.offset(i) = n1 + inc * i as f64;
        }
        crate::altrep::R_set_altrep_data2(x, v);
        Rf_unprotect(1);
    }
    expanded(x).std_data() as *mut c_void
}

unsafe extern "C" fn realseq_get_region(x: Sexp, i: isize, n: isize, buf: *mut f64) -> isize {
    let size = info(x).0;
    let ncopy = if size - i > n { n } else { size - i };
    for k in 0..ncopy.max(0) {
        *buf.offset(k) = realseq_elt(x, i + k);
    }
    ncopy
}

// R: altclasses.c:compact_realseq_Sum
unsafe extern "C" fn realseq_sum(x: Sexp, _narm: c_int) -> Sexp {
    let (size, n1, inc) = info(x);
    crate::list::Rf_ScalarReal((size as f64 / 2.0) * (n1 + n1 + inc * (size - 1) as f64))
}

unsafe extern "C" fn realseq_duplicate(x: Sexp, _deep: c_int) -> Sexp {
    let n = info(x).0;
    let v = Rf_protect(alloc_vector(REALSXP as u32, n));
    realseq_get_region(x, 0, n, v.std_data() as *mut f64);
    Rf_unprotect(1);
    v
}

unsafe extern "C" fn realseq_unserialize(_class: Sexp, state: Sexp) -> Sexp {
    let i = std::slice::from_raw_parts(state.std_data() as *const f64, 3);
    if i[2] != 1.0 && i[2] != -1.0 {
        crate::errors::error_str_owned(format!("compact sequences with increment {:.6} not supported yet", i[2]))
    }
    new_compact_realseq(i[0] as isize, i[1], i[2])
}

// ---- the classes -----------------------------------------------------------

unsafe fn intseq_class() -> ClassT {
    if let Some(c) = INTSEQ {
        return c;
    }
    use crate::altrep::*;
    let c = R_make_altinteger_class(c"compact_intseq".as_ptr(), c"base".as_ptr(), std::ptr::null_mut());
    R_set_altrep_Unserialize_method(c, intseq_unserialize as *const () as usize);
    R_set_altrep_Serialized_state_method(c, seq_serialized_state as *const () as usize);
    R_set_altrep_Duplicate_method(c, intseq_duplicate as *const () as usize);
    R_set_altrep_Coerce_method(c, intseq_coerce as *const () as usize);
    R_set_altrep_Length_method(c, intseq_length as *const () as usize);
    R_set_altvec_Dataptr_method(c, intseq_dataptr as *const () as usize);
    R_set_altvec_Dataptr_or_null_method(c, intseq_dataptr_or_null as *const () as usize);
    R_set_altinteger_Elt_method(c, intseq_elt as *const () as usize);
    R_set_altinteger_Get_region_method(c, intseq_get_region as *const () as usize);
    R_set_altinteger_Is_sorted_method(c, intseq_is_sorted as *const () as usize);
    R_set_altinteger_No_NA_method(c, seq_no_na as *const () as usize);
    R_set_altinteger_Sum_method(c, intseq_sum as *const () as usize);
    INTSEQ = Some(c);
    c
}

unsafe fn realseq_class() -> ClassT {
    if let Some(c) = REALSEQ {
        return c;
    }
    use crate::altrep::*;
    let c = R_make_altreal_class(c"compact_realseq".as_ptr(), c"base".as_ptr(), std::ptr::null_mut());
    R_set_altrep_Unserialize_method(c, realseq_unserialize as *const () as usize);
    R_set_altrep_Serialized_state_method(c, seq_serialized_state as *const () as usize);
    R_set_altrep_Duplicate_method(c, realseq_duplicate as *const () as usize);
    R_set_altrep_Length_method(c, intseq_length as *const () as usize);
    R_set_altvec_Dataptr_method(c, realseq_dataptr as *const () as usize);
    R_set_altvec_Dataptr_or_null_method(c, intseq_dataptr_or_null as *const () as usize);
    R_set_altreal_Elt_method(c, realseq_elt as *const () as usize);
    R_set_altreal_Get_region_method(c, realseq_get_region as *const () as usize);
    R_set_altreal_Is_sorted_method(c, intseq_is_sorted as *const () as usize);
    R_set_altreal_No_NA_method(c, seq_no_na as *const () as usize);
    R_set_altreal_Sum_method(c, realseq_sum as *const () as usize);
    REALSEQ = Some(c);
    c
}

unsafe fn new_seq(cls: ClassT, n: isize, n1: f64, inc: f64) -> Sexp {
    let info = Rf_protect(alloc_vector(REALSXP as u32, 3));
    let d = info.std_data() as *mut f64;
    *d = n as f64;
    *d.add(1) = n1;
    *d.add(2) = inc;
    let ans = crate::altrep::R_new_altrep(cls, info, R_NilValue);
    // not mutable: modification duplicates (altclasses.c MARK_NOT_MUTABLE)
    ans.set_refcnt(u16::MAX);
    Rf_unprotect(1);
    ans
}

// R: altclasses.c:new_compact_intseq
pub unsafe fn new_compact_intseq(n: isize, n1: c_int, inc: c_int) -> Sexp {
    if n == 1 {
        return crate::list::Rf_ScalarInteger(n1);
    }
    new_seq(intseq_class(), n, n1 as f64, inc as f64)
}

// R: altclasses.c:new_compact_realseq
pub unsafe fn new_compact_realseq(n: isize, n1: f64, inc: f64) -> Sexp {
    if n == 1 {
        return crate::list::Rf_ScalarReal(n1);
    }
    new_seq(realseq_class(), n, n1, inc)
}

/// A compact sequence object, as (int?, length, first, increment), unless
/// C expanded it (its data may have been written through DATAPTR).
pub unsafe fn compact_info(x: Sexp) -> Option<(bool, isize, f64, f64)> {
    if !x.is_altrep() || expanded(x) != R_NilValue {
        return None;
    }
    let c = crate::altrep::class_of(x);
    let int = matches!(INTSEQ, Some(k) if k.ptr == c);
    let real = matches!(REALSEQ, Some(k) if k.ptr == c);
    if !int && !real {
        return None;
    }
    let (n, n1, inc) = info(x);
    Some((int, n, n1, inc))
}
