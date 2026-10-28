//! memory.c: allocation, the protect stack, the precious list, R_alloc,
//! and the object accessors (Rinternals.h's function forms).

use crate::errors::{error_str, error_str_owned};
use crate::state::*;
use crate::util::type_to_char;
use robigo_core::types::*;
use robigo_core::{Rcomplex, Sexp, flags, gp};
use std::ffi::{c_char, c_int, c_uint, c_void};

#[inline(always)]
fn c() -> &'static mut robigo_core::Core {
    core()
}

/// GNU R's error for an accessor applied to the wrong type.
#[cold]
unsafe fn wrong_type(func: &str, want: &str, x: Sexp) -> ! {
    error_str_owned(format!("{func}() can only be applied to a '{want}', not a '{}'", type_to_char(x)))
}

// ---- allocation -------------------------------------------------------

// R: memory.c:allocVector3
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocVector(t: c_uint, n: isize) -> Sexp {
    alloc_vector(t, n)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocVector3(t: c_uint, n: isize, _allocator: *mut c_void) -> Sexp {
    alloc_vector(t, n)
}

pub unsafe fn alloc_vector(t: c_uint, n: isize) -> Sexp {
    if n < 0 {
        error_str("negative length vectors are not allowed")
    }
    let t = t as u8;
    match t {
        NILSXP => R_NilValue,
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | EXPRSXP | RAWSXP | CHARSXP => {
            c().alloc_vector(t, n as usize)
        }
        LISTSXP => Rf_allocList(n as c_int),
        LANGSXP => {
            if n == 0 {
                return R_NilValue;
            }
            let s = Rf_allocList(n as c_int);
            s.set_typ(LANGSXP);
            s
        }
        _ => error_str_owned(format!("invalid type/length ({}/{}) in vector allocation", type_to_char_t(t), n)),
    }
}

pub fn type_to_char_t(t: u8) -> &'static str {
    crate::util::type_name(t)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocList(n: c_int) -> Sexp {
    let mut r = R_NilValue;
    for _ in 0..n.max(0) {
        r = c().alloc_node(LISTSXP, R_NilValue, r, R_NilValue);
    }
    r
}

// R: memory.c:allocSExp
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocSExp(t: c_uint) -> Sexp {
    let nil = R_NilValue;
    c().alloc_node(t as u8, nil, nil, nil)
}

// R: memory.c:allocS4Object
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocS4Object() -> Sexp {
    let s = Rf_allocSExp(OBJSXP as c_uint);
    s.set_gp(s.gp() | gp::S4);
    s
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_allocObject() -> Sexp {
    Rf_allocSExp(OBJSXP as c_uint)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_cons(car: Sexp, cdr: Sexp) -> Sexp {
    c().alloc_node(LISTSXP, car, cdr, R_NilValue)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lcons(car: Sexp, cdr: Sexp) -> Sexp {
    c().alloc_node(LANGSXP, car, cdr, R_NilValue)
}

// R: array.c:allocMatrix
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocMatrix(t: c_uint, nrow: c_int, ncol: c_int) -> Sexp {
    if nrow < 0 || ncol < 0 {
        error_str("negative extents to matrix")
    }
    let s = alloc_vector(t, nrow as isize * ncol as isize);
    Rf_protect(s);
    let d = c().alloc_vector(INTSXP, 2);
    *(d.std_data() as *mut i32) = nrow;
    *(d.std_data() as *mut i32).add(1) = ncol;
    crate::attrib::Rf_setAttrib(s, R_DimSymbol, d);
    Rf_unprotect(1);
    s
}

// R: array.c:alloc3DArray
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_alloc3DArray(t: c_uint, nrow: c_int, ncol: c_int, nface: c_int) -> Sexp {
    if nrow < 0 || ncol < 0 || nface < 0 {
        error_str("negative extents to 3D array")
    }
    let s = alloc_vector(t, nrow as isize * ncol as isize * nface as isize);
    Rf_protect(s);
    let d = c().alloc_vector(INTSXP, 3);
    let p = d.std_data() as *mut i32;
    *p = nrow;
    *p.add(1) = ncol;
    *p.add(2) = nface;
    crate::attrib::Rf_setAttrib(s, R_DimSymbol, d);
    Rf_unprotect(1);
    s
}

// R: array.c:allocArray
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocArray(t: c_uint, dims: Sexp) -> Sexp {
    let nd = LENGTH(dims) as usize;
    let p = INTEGER(dims);
    let mut n: isize = 1;
    for i in 0..nd {
        n *= *p.add(i) as isize;
    }
    let s = alloc_vector(t, n);
    Rf_protect(s);
    crate::attrib::Rf_setAttrib(s, R_DimSymbol, Rf_duplicate(dims));
    Rf_unprotect(1);
    s
}

// ---- the protect stack (GNU R's layout, exported) --------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_protect(s: Sexp) -> Sexp {
    if R_PPStackTop >= R_PPStackSize {
        crate::errors::R_signal_protect_error()
    }
    *R_PPStack.add(R_PPStackTop as usize) = s;
    R_PPStackTop += 1;
    s
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_unprotect(n: c_int) {
    if R_PPStackTop >= n {
        R_PPStackTop -= n;
    } else {
        crate::errors::R_signal_unprotect_error()
    }
}

// R: memory.c:unprotect_ptr
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_unprotect_ptr(s: Sexp) {
    let mut i = R_PPStackTop;
    loop {
        if i == 0 {
            error_str("unprotect_ptr: pointer not found")
        }
        i -= 1;
        if *R_PPStack.add(i as usize) == s {
            break;
        }
    }
    i += 1;
    while i < R_PPStackTop {
        *R_PPStack.add(i as usize - 1) = *R_PPStack.add(i as usize);
        i += 1;
    }
    R_PPStackTop -= 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ProtectWithIndex(s: Sexp, pi: *mut c_int) {
    Rf_protect(s);
    *pi = R_PPStackTop - 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_Reprotect(s: Sexp, i: c_int) {
    if i >= R_PPStackTop || i < 0 {
        crate::errors::R_signal_reprotect_error(i)
    }
    *R_PPStack.add(i as usize) = s;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_PreserveObject(x: Sexp) {
    c().preserve(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ReleaseObject(x: Sexp) {
    c().release(x)
}

// R: memory.c:R_NewPreciousMSet & co. A multi-set is CONS(store,
// npreserved) tagged with the initial size; it protects its members as
// long as it is itself protected.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_NewPreciousMSet(initial: c_int) -> Sexp {
    let np = c().alloc_vector(INTSXP, 1);
    *(np.std_data() as *mut i32) = 0;
    let mset = Rf_protect(Rf_cons(R_NilValue, np));
    if initial < 0 {
        error_str("'initialSize' must be non-negative")
    }
    let isize = crate::list::Rf_ScalarInteger(initial);
    SET_TAG(mset, isize);
    Rf_unprotect(1);
    mset
}

unsafe fn check_mset(mset: Sexp) {
    let store = mset.car();
    let np = mset.cdr();
    let isize = mset.tag();
    if (store != R_NilValue && store.typ() != VECSXP)
        || np.typ() != INTSXP
        || xlength(np) != 1
        || isize.typ() != INTSXP
        || xlength(isize) != 1
    {
        error_str("Invalid mset")
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_PreserveInMSet(x: Sexp, mset: Sexp) {
    if x == R_NilValue || x.typ() == SYMSXP {
        return;
    }
    Rf_protect(x);
    check_mset(mset);
    let mut store = mset.car();
    let n = mset.cdr().std_data() as *mut i32;
    if store == R_NilValue {
        let mut newsize = *(mset.tag().std_data() as *const i32) as isize;
        if newsize == 0 {
            newsize = 4;
        }
        store = c().alloc_vector(VECSXP, newsize as usize);
        SETCAR(mset, store);
    }
    let size = xlength(store);
    if *n as isize == size {
        let newsize = 2 * size;
        if newsize >= i32::MAX as isize {
            error_str("Multi-set overflow")
        }
        let ns = Rf_protect(c().alloc_vector(VECSXP, newsize as usize));
        for i in 0..size {
            SET_VECTOR_ELT(ns, i, VECTOR_ELT(store, i));
        }
        SETCAR(mset, ns);
        Rf_unprotect(1);
        store = ns;
    }
    Rf_unprotect(1);
    SET_VECTOR_ELT(store, *n as isize, x);
    *n += 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ReleaseFromMSet(x: Sexp, mset: Sexp) {
    if x == R_NilValue || x.typ() == SYMSXP {
        return;
    }
    check_mset(mset);
    let store = mset.car();
    if store == R_NilValue {
        return;
    }
    let n = mset.cdr().std_data() as *mut i32;
    let mut i = *n as isize - 1;
    while i >= 0 {
        if VECTOR_ELT(store, i) == x {
            while i < *n as isize - 1 {
                SET_VECTOR_ELT(store, i, VECTOR_ELT(store, i + 1));
                i += 1;
            }
            SET_VECTOR_ELT(store, i, R_NilValue);
            *n -= 1;
            return;
        }
        i -= 1;
    }
}

// R: memory.c:R_ReleaseMSet
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ReleaseMSet(mset: Sexp, keep_size: c_int) {
    check_mset(mset);
    let store = mset.car();
    if store == R_NilValue {
        return;
    }
    let n = mset.cdr().std_data() as *mut i32;
    if xlength(store) <= keep_size as isize {
        for i in 0..*n as isize {
            SET_VECTOR_ELT(store, i, R_NilValue);
        }
    } else {
        SETCAR(mset, R_NilValue);
    }
    *n = 0;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_gc() {
    c().collect();
    crate::util::safe_point();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_gc_running() -> c_int {
    0
}

// ---- R_alloc's stack ----------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vmaxget() -> *mut c_void {
    st().vstack.0 as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vmaxset(v: *const c_void) {
    st().vstack = if v.is_null() { R_NilValue } else { Sexp(v as *mut u8) };
}

// R: memory.c:R_alloc
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_alloc(nelem: usize, eltsize: c_int) -> *mut c_char {
    let size = nelem.wrapping_mul(eltsize as usize);
    let dsize = nelem as f64 * eltsize as f64;
    if dsize > 0.0 {
        if dsize > isize::MAX as f64 {
            error_str_owned(format!("cannot allocate memory block of size {:.0} Tb", dsize / 1024f64.powi(4)))
        }
        let s = c().alloc_vector(RAWSXP, size + 1);
        s.set_attrib_raw(st().vstack);
        st().vstack = s;
        s.std_data() as *mut c_char
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_allocLD(nelem: usize) -> *mut c_void {
    let p = R_alloc(nelem + 1, 16) as usize;
    ((p + 15) & !15) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn S_alloc(nelem: libc::c_long, eltsize: c_int) -> *mut c_char {
    let size = nelem as usize * eltsize as usize;
    let p = R_alloc(nelem as usize, eltsize);
    if !p.is_null() {
        std::ptr::write_bytes(p, 0, size);
    }
    p
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn S_realloc(p: *mut c_char, new: libc::c_long, old: libc::c_long, size: c_int) -> *mut c_char {
    if new <= old {
        return p;
    }
    let q = R_alloc(new as usize, size);
    let nold = old as usize * size as usize;
    if nold > 0 {
        std::ptr::copy_nonoverlapping(p, q, nold);
    }
    std::ptr::write_bytes(q.add(nold), 0, new as usize * size as usize - nold);
    q
}

// R: memory.c:R_chk_calloc & co
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_chk_calloc(nelem: usize, elsize: usize) -> *mut c_void {
    let p = libc::calloc(nelem.max(1), elsize.max(1));
    if p.is_null() {
        error_str_owned(format!("'R_Calloc' could not allocate memory ({} of {} bytes)", nelem, elsize))
    }
    p
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_chk_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    let p = if ptr.is_null() { libc::malloc(size) } else { libc::realloc(ptr, size) };
    if p.is_null() {
        error_str_owned(format!("'R_Realloc' could not re-allocate memory ({} bytes)", size))
    }
    p
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_chk_free(ptr: *mut c_void) {
    if !ptr.is_null() {
        libc::free(ptr);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_chk_memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    if n > 0 {
        std::ptr::copy(src as *const u8, dest as *mut u8, n);
    }
    dest
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_chk_memset(dest: *mut c_void, v: c_int, n: usize) -> *mut c_void {
    if n > 0 {
        std::ptr::write_bytes(dest as *mut u8, v as u8, n);
    }
    dest
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_malloc_gc(n: usize) -> *mut c_void {
    libc::malloc(n)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_calloc_gc(n: usize, s: usize) -> *mut c_void {
    libc::calloc(n, s)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_realloc_gc(p: *mut c_void, n: usize) -> *mut c_void {
    libc::realloc(p, n)
}

/// Stack usage of the R thread (errors.c: R_CStackDir * (start - sp)).
#[inline(always)]
fn stack_usage() -> isize {
    let marker = 0u8;
    let sp = std::hint::black_box(&marker) as *const u8 as usize;
    unsafe { R_CStackDir as isize * (R_CStackStart as isize - sp as isize) }
}

// R: errors.c:R_SignalCStackOverflow (the condition is reported without a call)
unsafe fn signal_cstack_overflow(usage: isize) -> ! {
    crate::errors::errorcall_owned(R_NilValue, format!("C stack usage  {usage} is too close to the limit"))
}

// R: errors.c:R_CheckStack
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_CheckStack() {
    let usage = stack_usage();
    if R_CStackLimit != usize::MAX && usage > R_CStackLimit as isize {
        signal_cstack_overflow(usage)
    }
}

// R: errors.c:R_CheckStack2
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_CheckStack2(extra: usize) {
    let usage = stack_usage();
    if (isize::MAX - usage) < extra as isize {
        signal_cstack_overflow(isize::MAX)
    }
    let usage = usage + extra as isize;
    if R_CStackLimit != usize::MAX && usage > R_CStackLimit as isize {
        signal_cstack_overflow(usage)
    }
}

// ---- header accessors ------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TYPEOF(x: Sexp) -> c_int {
    x.typ() as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_TYPEOF(x: Sexp, t: c_int) {
    x.set_typ(t as u8)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OBJECT(x: Sexp) -> c_int {
    x.has_flag(flags::OBJECT) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_OBJECT(x: Sexp, v: c_int) {
    x.set_flag(flags::OBJECT, v != 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_S4_OBJECT(x: Sexp) -> c_int {
    (x.gp() & gp::S4 != 0) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_S4_OBJECT(x: Sexp) {
    x.set_gp(x.gp() | gp::S4)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn UNSET_S4_OBJECT(x: Sexp) {
    x.set_gp(x.gp() & !gp::S4)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LEVELS(x: Sexp) -> c_int {
    x.gp() as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SETLEVELS(x: Sexp, v: c_int) -> c_int {
    x.set_gp(v as u16);
    v
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTREP(x: Sexp) -> c_int {
    x.is_altrep() as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ATTRIB(x: Sexp) -> Sexp {
    if x.typ() == CHARSXP { R_NilValue } else { x.attrib_raw() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ANY_ATTRIB(x: Sexp) -> c_int {
    (ATTRIB(x) != R_NilValue) as c_int
}

// R: memory.c:SET_ATTRIB
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_ATTRIB(x: Sexp, v: Sexp) {
    if v.typ() != LISTSXP && v.typ() != NILSXP {
        error_str_owned(format!("value of 'SET_ATTRIB' must be a pairlist or NULL, not a '{}'", type_to_char(v)))
    }
    fix_refcnt(x, x.attrib_raw(), v);
    x.set_attrib_raw(v);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CLEAR_ATTRIB(x: Sexp) {
    SET_ATTRIB(x, R_NilValue);
    SET_OBJECT(x, 0);
    UNSET_S4_OBJECT(x);
}

// R: memory.c:DUPLICATE_ATTRIB
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DUPLICATE_ATTRIB(to: Sexp, from: Sexp) {
    SET_ATTRIB(to, Rf_duplicate(ATTRIB(from)));
    SET_OBJECT(to, OBJECT(from));
    if IS_S4_OBJECT(from) != 0 { SET_S4_OBJECT(to) } else { UNSET_S4_OBJECT(to) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SHALLOW_DUPLICATE_ATTRIB(to: Sexp, from: Sexp) {
    SET_ATTRIB(to, Rf_shallow_duplicate(ATTRIB(from)));
    SET_OBJECT(to, OBJECT(from));
    if IS_S4_OBJECT(from) != 0 { SET_S4_OBJECT(to) } else { UNSET_S4_OBJECT(to) }
}

// ---- reference counts ------------------------------------------------------

#[inline]
pub unsafe fn fix_refcnt(x: Sexp, old: Sexp, new: Sexp) {
    if x.has_flag(flags::IMMORTAL) {
        c().write_barrier(x);
    }
    if x.tracks_refs() && old != new {
        if !old.is_null() {
            old.decref();
        }
        if !new.is_null() {
            new.incref();
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn REFCNT(x: Sexp) -> c_int {
    if x.has_flag(flags::IMMORTAL) { 65535 } else { x.refcnt() as c_int }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn NAMED(x: Sexp) -> c_int {
    REFCNT(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_NAMED(_x: Sexp, _v: c_int) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn MAYBE_SHARED(x: Sexp) -> c_int {
    (REFCNT(x) > 1) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn MAYBE_REFERENCED(x: Sexp) -> c_int {
    (REFCNT(x) > 0) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn NO_REFERENCES(x: Sexp) -> c_int {
    (REFCNT(x) == 0) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn MARK_NOT_MUTABLE(x: Sexp) {
    x.set_refcnt(u16::MAX)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ENSURE_NAMEDMAX(x: Sexp) {
    MARK_NOT_MUTABLE(x)
}

// ---- vector accessors --------------------------------------------------------

#[inline]
fn nvec(t: u8) -> bool {
    // memory.c CHK2: types LENGTH and friends refuse
    !matches!(
        t,
        NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | EXPRSXP | RAWSXP | CHARSXP | WEAKREFSXP
    ) && t != LISTSXP
        && t != LANGSXP
        && t != DOTSXP
}

/// The length of a vector, ALTREP-aware.
#[inline]
pub unsafe fn xlength(x: Sexp) -> isize {
    if x.is_altrep() { crate::util::altrep_length(x) } else { x.std_len() }
}

unsafe fn chk2(x: Sexp) {
    if nvec(x.typ()) {
        error_str_owned(format!("LENGTH or similar applied to {} object", type_to_char(x)))
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LENGTH(x: Sexp) -> c_int {
    if x == R_NilValue {
        return 0;
    }
    chk2(x);
    if matches!(x.typ(), LISTSXP | LANGSXP | DOTSXP) {
        return Rf_length(x);
    }
    let n = xlength(x);
    if n > i32::MAX as isize {
        crate::errors::R_BadLongVector(x, c"memory.c".as_ptr(), 0)
    }
    n as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn XLENGTH(x: Sexp) -> isize {
    chk2(x);
    if matches!(x.typ(), LISTSXP | LANGSXP | DOTSXP) {
        return Rf_length(x) as isize;
    }
    xlength(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_XLENGTH(x: Sexp) -> isize {
    XLENGTH(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TRUELENGTH(x: Sexp) -> isize {
    chk2(x);
    if x.is_altrep() { 0 } else { x.truelen() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_TRUELENGTH(x: Sexp, v: isize) {
    chk2(x);
    x.set_truelen(v)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SETLENGTH(x: Sexp, v: isize) {
    if x.is_altrep() {
        error_str("SETLENGTH() cannot be applied to an ALTVEC object.")
    }
    if !crate::util::is_vector(x) {
        error_str_owned(format!("SETLENGTH() can only be applied to a standard vector, not a '{}'", type_to_char(x)))
    }
    x.set_std_len(v)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_LONG_VEC(x: Sexp) -> c_int {
    chk2(x);
    (xlength(x) > i32::MAX as isize) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_GROWABLE(x: Sexp) -> c_int {
    (x.gp() & gp::GROWABLE != 0 && xlength(x) < x.truelen()) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_GROWABLE_BIT(x: Sexp) {
    x.set_gp(x.gp() | gp::GROWABLE)
}

/// DATAPTR with GNU R's zero-length convention.
#[inline]
pub unsafe fn dataptr(x: Sexp) -> *mut u8 {
    if x.is_altrep() {
        return crate::util::altrep_dataptr(x, true);
    }
    if x.std_len() == 0 && x.typ() != CHARSXP {
        return 1 as *mut u8;
    }
    x.std_data()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DATAPTR(x: Sexp) -> *mut c_void {
    dataptr(x) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DATAPTR_RO(x: Sexp) -> *const c_void {
    if x.is_altrep() {
        return crate::util::altrep_dataptr(x, false) as *const c_void;
    }
    dataptr(x) as *const c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DATAPTR_OR_NULL(x: Sexp) -> *const c_void {
    if x.is_altrep() {
        return crate::util::altrep_dataptr_or_null(x) as *const c_void;
    }
    dataptr(x) as *const c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn STDVEC_DATAPTR(x: Sexp) -> *mut c_void {
    if x.is_altrep() {
        error_str("cannot get STDVEC_DATAPTR from ALTREP object")
    }
    dataptr(x) as *mut c_void
}

macro_rules! typed_ptr {
    ($name:ident, $ro:ident, $ornull:ident, $t:ty, $fname:expr, $want:expr, $($types:pat_param)|+) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(x: Sexp) -> *mut $t {
            if !matches!(x.typ(), $($types)|+) {
                wrong_type($fname, $want, x)
            }
            dataptr(x) as *mut $t
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $ro(x: Sexp) -> *const $t {
            if !matches!(x.typ(), $($types)|+) {
                wrong_type($fname, $want, x)
            }
            DATAPTR_RO(x) as *const $t
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $ornull(x: Sexp) -> *const $t {
            if !matches!(x.typ(), $($types)|+) {
                wrong_type($fname, $want, x)
            }
            DATAPTR_OR_NULL(x) as *const $t
        }
    };
}

typed_ptr!(LOGICAL, LOGICAL_RO, LOGICAL_OR_NULL, c_int, "LOGICAL", "logical", LGLSXP);
typed_ptr!(INTEGER, INTEGER_RO, INTEGER_OR_NULL, c_int, "INTEGER", "integer", INTSXP | LGLSXP);
typed_ptr!(REAL, REAL_RO, REAL_OR_NULL, f64, "REAL", "numeric", REALSXP);
typed_ptr!(COMPLEX, COMPLEX_RO, COMPLEX_OR_NULL, Rcomplex, "COMPLEX", "complex", CPLXSXP);
typed_ptr!(RAW, RAW_RO, RAW_OR_NULL, u8, "RAW", "raw", RAWSXP);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn STRING_PTR(x: Sexp) -> *mut Sexp {
    if x.typ() != STRSXP {
        wrong_type("STRING_PTR", "character", x)
    }
    dataptr(x) as *mut Sexp
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn STRING_PTR_RO(x: Sexp) -> *const Sexp {
    if x.typ() != STRSXP {
        wrong_type("STRING_PTR_RO", "character", x)
    }
    DATAPTR_RO(x) as *const Sexp
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn VECTOR_PTR_RO(x: Sexp) -> *const Sexp {
    if x.typ() != VECSXP {
        wrong_type("VECTOR_PTR_RO", "list", x)
    }
    DATAPTR_RO(x) as *const Sexp
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn VECTOR_PTR(_x: Sexp) -> *mut Sexp {
    error_str("not safe to return vector pointer")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_CHAR(x: Sexp) -> *const c_char {
    if x.typ() != CHARSXP {
        wrong_type("CHAR", "CHARSXP", x)
    }
    x.std_data() as *const c_char
}

// R: memory.c:STRING_ELT
#[unsafe(no_mangle)]
pub unsafe extern "C" fn STRING_ELT(x: Sexp, i: isize) -> Sexp {
    if x.typ() == STRSXP && !x.is_altrep() {
        return *(x.std_data() as *const Sexp).offset(i);
    }
    string_elt_slow(x, i)
}

#[cold]
#[inline(never)]
unsafe fn string_elt_slow(x: Sexp, i: isize) -> Sexp {
    if x.typ() != STRSXP {
        wrong_type("STRING_ELT", "character vector", x)
    }
    if x.is_altrep() {
        return crate::util::altstring_elt(x, i);
    }
    *(x.std_data() as *const Sexp).offset(i)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn VECTOR_ELT(x: Sexp, i: isize) -> Sexp {
    if matches!(x.typ(), VECSXP | EXPRSXP) && !x.is_altrep() {
        return *(x.std_data() as *const Sexp).offset(i);
    }
    vector_elt_slow(x, i)
}

#[cold]
#[inline(never)]
unsafe fn vector_elt_slow(x: Sexp, i: isize) -> Sexp {
    if !matches!(x.typ(), VECSXP | EXPRSXP | WEAKREFSXP) {
        wrong_type("VECTOR_ELT", "list", x)
    }
    if x.is_altrep() {
        let a = crate::util::altlist_elt(x, i);
        MARK_NOT_MUTABLE(a);
        return a;
    }
    *(x.std_data() as *const Sexp).offset(i)
}

/// The store of a child into a standard vector slot, with GNU R's
/// reference counting, when nothing else is needed (no calls: the hot
/// accessors stay free of a stack frame).
#[inline(always)]
unsafe fn store_slot(x: Sexp, p: *mut Sexp, v: Sexp) {
    let old = *p;
    if old != v && x.tracks_refs() {
        old.decref();
        v.incref();
    }
    *p = v;
}

#[inline(always)]
unsafe fn plain_target(x: Sexp, i: isize) -> bool {
    !x.is_altrep() && !x.has_flag(flags::IMMORTAL) && (i as usize) < x.std_len() as usize
}

// R: memory.c:SET_STRING_ELT
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_STRING_ELT(x: Sexp, i: isize, v: Sexp) {
    if x.typ() == STRSXP && v.typ() == CHARSXP && plain_target(x, i) {
        return store_slot(x, (x.std_data() as *mut Sexp).offset(i), v);
    }
    set_string_elt_slow(x, i, v)
}

#[cold]
#[inline(never)]
unsafe fn set_string_elt_slow(x: Sexp, i: isize, v: Sexp) {
    if x.typ() != STRSXP {
        wrong_type("SET_STRING_ELT", "character vector", x)
    }
    if v.typ() != CHARSXP {
        error_str_owned(format!("Value of SET_STRING_ELT() must be a 'CHARSXP' not a '{}'", type_to_char(v)))
    }
    let n = xlength(x);
    if i < 0 || i >= n {
        error_str_owned(format!("attempt to set index {i}/{n} in SET_STRING_ELT"))
    }
    if x.is_altrep() {
        return crate::util::altstring_set_elt(x, i, v);
    }
    let p = (x.std_data() as *mut Sexp).offset(i);
    fix_refcnt(x, *p, v);
    *p = v;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_VECTOR_ELT(x: Sexp, i: isize, v: Sexp) -> Sexp {
    if matches!(x.typ(), VECSXP | EXPRSXP) && plain_target(x, i) {
        store_slot(x, (x.std_data() as *mut Sexp).offset(i), v);
        return v;
    }
    set_vector_elt_slow(x, i, v)
}

#[cold]
#[inline(never)]
unsafe fn set_vector_elt_slow(x: Sexp, i: isize, v: Sexp) -> Sexp {
    if !matches!(x.typ(), VECSXP | EXPRSXP | WEAKREFSXP) {
        wrong_type("SET_VECTOR_ELT", "list", x)
    }
    let n = xlength(x);
    if i < 0 || i >= n {
        error_str_owned(format!("attempt to set index {i}/{n} in SET_VECTOR_ELT"))
    }
    if x.is_altrep() {
        crate::util::altlist_set_elt(x, i, v);
        return v;
    }
    let p = (x.std_data() as *mut Sexp).offset(i);
    fix_refcnt(x, *p, v);
    *p = v;
    v
}

macro_rules! elt_accessors {
    ($get:ident, $set:ident, $ptr:ident, $t:ty, $elt:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $get(x: Sexp, i: isize) -> $t {
            if x.is_altrep() {
                return crate::util::$elt(x, i);
            }
            *$ptr(x).offset(i)
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $set(x: Sexp, i: isize, v: $t) {
            *$ptr(x).offset(i) = v;
        }
    };
}

elt_accessors!(INTEGER_ELT, SET_INTEGER_ELT, INTEGER, c_int, altinteger_elt);
elt_accessors!(LOGICAL_ELT, SET_LOGICAL_ELT, LOGICAL, c_int, altlogical_elt);
elt_accessors!(REAL_ELT, SET_REAL_ELT, REAL, f64, altreal_elt);
elt_accessors!(COMPLEX_ELT, SET_COMPLEX_ELT, COMPLEX, Rcomplex, altcomplex_elt);
elt_accessors!(RAW_ELT, SET_RAW_ELT, RAW, u8, altraw_elt);

macro_rules! get_region {
    ($name:ident, $t:ty, $ptr:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(x: Sexp, i: isize, n: isize, buf: *mut $t) -> isize {
            let len = xlength(x);
            let ncopy = if len - i > n { n } else { len - i };
            let p = $ptr(x);
            for k in 0..ncopy.max(0) {
                *buf.offset(k) = *p.offset(i + k);
            }
            ncopy
        }
    };
}

get_region!(INTEGER_GET_REGION, c_int, INTEGER);
get_region!(LOGICAL_GET_REGION, c_int, LOGICAL);
get_region!(REAL_GET_REGION, f64, REAL);
get_region!(COMPLEX_GET_REGION, Rcomplex, COMPLEX);
get_region!(RAW_GET_REGION, u8, RAW);

// ---- nodes ----------------------------------------------------------------

/// Field `k` (0 car, 1 cdr, 2 tag) of a node; HOST proxies (robigo
/// environments, closures, promises before G3) ask the host.
#[inline]
pub unsafe fn field(x: Sexp, k: c_int) -> Sexp {
    if x.has_flag(flags::HOST) {
        let ks = c().alloc_vector(INTSXP, 1);
        *(ks.std_data() as *mut i32) = k;
        return crate::host::op("field\0", &[x, ks]);
    }
    match k {
        0 => x.car(),
        1 => x.cdr(),
        _ => x.tag(),
    }
}

macro_rules! node_get {
    ($($name:ident = $($step:literal),+;)*) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(x: Sexp) -> Sexp {
                let mut v = x;
                $(v = field(v, $step);)+
                v
            }
        )*
    };
}

// steps applied left to right: CADR = car(cdr(x)) is [1, 0]
node_get! {
    CAR = 0;
    CDR = 1;
    TAG = 2;
    CAAR = 0, 0;
    CDAR = 0, 1;
    CADR = 1, 0;
    CDDR = 1, 1;
    CDDDR = 1, 1, 1;
    CADDR = 1, 1, 0;
    CADDDR = 1, 1, 1, 0;
    CAD4R = 1, 1, 1, 1, 0;
    CAD5R = 1, 1, 1, 1, 1, 0;
    FORMALS = 0;
    BODY = 1;
    CLOENV = 2;
    PRINTNAME = 0;
    SYMVALUE = 1;
    INTERNAL = 2;
    FRAME = 0;
    ENCLOS = 1;
    HASHTAB = 2;
    PRVALUE = 0;
    PRCODE = 1;
    PRENV = 2;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CAR0(x: Sexp) -> Sexp {
    x.car()
}

/// R's CHKCONS for the setters: the node must be cons-shaped.
unsafe fn chkcons(x: Sexp) {
    if x == R_NilValue || x.is_null() {
        error_str("bad value")
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SETCAR(x: Sexp, y: Sexp) -> Sexp {
    if x == R_NilValue || x.is_null() {
        error_str("bad value")
    }
    fix_refcnt(x, x.car(), y);
    x.set_car_raw(y);
    y
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SETCDR(x: Sexp, y: Sexp) -> Sexp {
    if x == R_NilValue || x.is_null() {
        error_str("bad value")
    }
    fix_refcnt(x, x.cdr(), y);
    x.set_cdr_raw(y);
    y
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_TAG(x: Sexp, y: Sexp) {
    chkcons(x);
    fix_refcnt(x, x.tag(), y);
    x.set_tag_raw(y);
}

macro_rules! setcxr {
    ($($name:ident = $($step:ident).*;)*) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(x: Sexp, y: Sexp) -> Sexp {
                let cell = x$(.$step())*;
                if cell == R_NilValue || cell.is_null() {
                    error_str("bad value")
                }
                SETCAR(cell, y)
            }
        )*
    };
}

setcxr! {
    SETCADR = cdr;
    SETCADDR = cdr.cdr;
    SETCADDDR = cdr.cdr.cdr;
    SETCAD4R = cdr.cdr.cdr.cdr;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_FORMALS(x: Sexp, v: Sexp) {
    SETCAR(x, v);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_BODY(x: Sexp, v: Sexp) {
    SETCDR(x, v);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_CLOENV(x: Sexp, v: Sexp) {
    SET_TAG(x, v);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_PRVALUE(x: Sexp, v: Sexp) {
    SETCAR(x, v);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_PRCODE(x: Sexp, v: Sexp) {
    SETCDR(x, v);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_PRENV(x: Sexp, v: Sexp) {
    SET_TAG(x, v);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn MISSING(x: Sexp) -> c_int {
    (x.gp() & 15) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_MISSING(x: Sexp, v: c_int) {
    x.set_gp((x.gp() & !15) | (v as u16 & 15))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RDEBUG(x: Sexp) -> c_int {
    (x.extra() & 1) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_RDEBUG(x: Sexp, v: c_int) {
    x.set_extra((x.extra() & !1) | (v != 0) as u16)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RTRACE(x: Sexp) -> c_int {
    ((x.extra() >> 1) & 1) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn NOJIT(x: Sexp) -> c_int {
    ((x.extra() >> 2) & 1) as c_int
}

// ---- length (Rinlinedfuns.h) ------------------------------------------------

// R: Rinlinedfuns.h:length
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_length(s: Sexp) -> c_int {
    match s.typ() {
        NILSXP => 0,
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | CHARSXP | VECSXP | EXPRSXP | RAWSXP => LENGTH(s),
        LISTSXP | LANGSXP | DOTSXP => {
            let mut i = 0;
            let mut p = s;
            while !p.is_null() && p != R_NilValue {
                i += 1;
                p = p.cdr();
            }
            i
        }
        ENVSXP => crate::util::Rf_envlength(s),
        _ => 1,
    }
}

// R: Rinlinedfuns.h:xlength
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_xlength(s: Sexp) -> isize {
    match s.typ() {
        NILSXP => 0,
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | CHARSXP | VECSXP | EXPRSXP | RAWSXP => xlength(s),
        LISTSXP | LANGSXP | DOTSXP => Rf_length(s) as isize,
        ENVSXP => crate::util::Rf_envxlength(s),
        _ => 1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LENGTH_EX(x: Sexp, _file: *const c_char, _line: c_int) -> c_int {
    LENGTH(x)
}

// ---- duplication (duplicate.c) -------------------------------------------------

unsafe fn duplicate1(s: Sexp, deep: bool) -> Sexp {
    let nil = R_NilValue;
    let t = s.typ();
    match t {
        NILSXP | SYMSXP | ENVSXP | SPECIALSXP | BUILTINSXP | EXTPTRSXP | BCODESXP | WEAKREFSXP | CHARSXP | PROMSXP => {
            return s;
        }
        _ => {}
    }
    if s.has_flag(flags::HOST) {
        return s;
    }
    let dup_child = |x: Sexp| if deep { duplicate1(x, true) } else { x };
    let t_out;
    let r = match t {
        CLOSXP => {
            let r = c().alloc_node(CLOSXP, s.car(), s.cdr(), s.tag());
            t_out = r;
            t_out
        }
        LISTSXP | LANGSXP | DOTSXP => {
            // duplicate the spine (and the cars when deep)
            Rf_protect(s);
            let head = c().alloc_node(t, nil, nil, nil);
            Rf_protect(head);
            let mut src = s;
            let mut dst = head;
            loop {
                let car = dup_child(src.car());
                SETCAR(dst, car);
                SET_TAG(dst, src.tag());
                dst.set_gp(src.gp());
                let a = src.attrib_raw();
                if a != nil {
                    SET_ATTRIB(dst, duplicate1(a, deep));
                }
                dst.set_flags((dst.flags() & !flags::OBJECT) | (src.flags() & flags::OBJECT));
                src = src.cdr();
                if src == nil || src.typ() != t && !matches!(src.typ(), LISTSXP | LANGSXP | DOTSXP) {
                    if src != nil {
                        SETCDR(dst, dup_child(src));
                    }
                    break;
                }
                let next = c().alloc_node(LISTSXP, nil, nil, nil);
                SETCDR(dst, next);
                dst = next;
            }
            Rf_unprotect(2);
            return head;
        }
        LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP | STRSXP | VECSXP | EXPRSXP => {
            let n = xlength(s);
            Rf_protect(s);
            let r = c().alloc_vector(t, n as usize);
            if n > 0 {
                let src = dataptr(s) as *const u8;
                match t {
                    STRSXP => {
                        std::ptr::copy_nonoverlapping(src, r.std_data(), n as usize * 8);
                        let d = r.std_data() as *const Sexp;
                        for i in 0..n as usize {
                            (*d.add(i)).incref();
                        }
                    }
                    VECSXP | EXPRSXP => {
                        Rf_protect(r);
                        for i in 0..n {
                            let e = VECTOR_ELT(s, i);
                            SET_VECTOR_ELT(r, i, dup_child(e));
                        }
                        Rf_unprotect(1);
                    }
                    _ => std::ptr::copy_nonoverlapping(src, r.std_data(), n as usize * robigo_core::elt_size(t)),
                }
            }
            Rf_unprotect(1);
            if s.gp() & gp::GROWABLE != 0 {
                // a duplicate is never growable
            }
            r
        }
        OBJSXP => {
            let r = c().alloc_node(OBJSXP, nil, nil, nil);
            r
        }
        _ => error_str("unimplemented type in duplicate"),
    };
    Rf_protect(r);
    let a = s.attrib_raw();
    if a != nil {
        SET_ATTRIB(r, duplicate1(a, deep));
    }
    r.set_flags((r.flags() & !flags::OBJECT) | (s.flags() & flags::OBJECT));
    r.set_gp((r.gp() & !gp::S4) | (s.gp() & gp::S4));
    Rf_unprotect(1);
    r
}

// R: duplicate.c:duplicate
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_duplicate(s: Sexp) -> Sexp {
    if s.is_altrep() {
        return crate::util::altrep_duplicate(s, true);
    }
    duplicate1(s, true)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_shallow_duplicate(s: Sexp) -> Sexp {
    if s.is_altrep() {
        return crate::util::altrep_duplicate(s, false);
    }
    duplicate1(s, false)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lazy_duplicate(s: Sexp) -> Sexp {
    match s.typ() {
        NILSXP | SYMSXP | ENVSXP | SPECIALSXP | BUILTINSXP | EXTPTRSXP | BCODESXP | WEAKREFSXP | CHARSXP | PROMSXP => {}
        _ => MARK_NOT_MUTABLE(s),
    }
    s
}

// R: duplicate.c:copyVector
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_copyVector(s: Sexp, t: Sexp) {
    let st_ = s.typ();
    if st_ != t.typ() {
        error_str("vector types do not match in copyVector")
    }
    let ns = xlength(s);
    let nt = xlength(t);
    if nt == 0 {
        return;
    }
    for i in 0..ns {
        let k = i % nt;
        match st_ {
            STRSXP => SET_STRING_ELT(s, i, STRING_ELT(t, k)),
            VECSXP | EXPRSXP => {
                SET_VECTOR_ELT(s, i, Rf_lazy_duplicate(VECTOR_ELT(t, k)));
            }
            LGLSXP | INTSXP => *INTEGER(s).offset(i) = *INTEGER(t).offset(k),
            REALSXP => *REAL(s).offset(i) = *REAL(t).offset(k),
            CPLXSXP => *COMPLEX(s).offset(i) = *COMPLEX(t).offset(k),
            RAWSXP => *RAW(s).offset(i) = *RAW(t).offset(k),
            _ => error_str("unimplemented type in copyVector"),
        }
    }
}

// R: duplicate.c:copyMatrix
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_copyMatrix(s: Sexp, t: Sexp, byrow: c_int) {
    if byrow == 0 {
        return Rf_copyVector(s, t);
    }
    let nr = crate::util::Rf_nrows(s) as isize;
    let nc = crate::util::Rf_ncols(s) as isize;
    let nt = xlength(t);
    let mut k = 0isize;
    for i in 0..nr {
        for j in 0..nc {
            let to = i + j * nr;
            let from = k % nt;
            match s.typ() {
                STRSXP => SET_STRING_ELT(s, to, STRING_ELT(t, from)),
                VECSXP | EXPRSXP => {
                    SET_VECTOR_ELT(s, to, Rf_lazy_duplicate(VECTOR_ELT(t, from)));
                }
                LGLSXP | INTSXP => *INTEGER(s).offset(to) = *INTEGER(t).offset(from),
                REALSXP => *REAL(s).offset(to) = *REAL(t).offset(from),
                CPLXSXP => *COMPLEX(s).offset(to) = *COMPLEX(t).offset(from),
                RAWSXP => *RAW(s).offset(to) = *RAW(t).offset(from),
                _ => error_str("unimplemented type in copyMatrix"),
            }
            k += 1;
        }
    }
}
