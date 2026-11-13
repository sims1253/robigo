//! memory.c: allocation, the protect stack, the precious list, R_alloc,
//! and the object accessors (Rinternals.h's function forms).

use crate::errors::{Res, Unwind, error_str, ret};
use crate::state::*;
use crate::util::type_to_char;
use robigo_core::types::*;
use robigo_core::{Rcomplex, Sexp, flags, gp};
use std::ffi::{c_char, c_int, c_uint, c_void};

#[inline(always)]
unsafe fn c() -> &'static mut robigo_core::Core {
    core()
}

/// GNU R's error for an accessor applied to the wrong type.
#[cold]
unsafe fn wrong_type(func: &str, want: &str, x: Sexp) -> crate::errors::Unwind {
    error_str(&format!("{func}() can only be applied to a '{want}', not a '{}'", type_to_char(x)))
}

// ---- allocation -------------------------------------------------------

// R: memory.c:allocVector3
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocVector(t: c_uint, n: isize) -> Sexp {
    // the common cases without the `Res` (a small vector always has room)
    if n >= 0 && matches!(t as u8, LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | EXPRSXP | RAWSXP) {
        if n <= 256 {
            return c().alloc_vector(t as u8, n as usize);
        }
        match c().try_alloc_vector(t as u8, n as usize) {
            Some(x) => return x,
            None => crate::errors::throw(cannot_allocate(t as u8, n)),
        }
    }
    ret(alloc_vector(t, n))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocVector3(t: c_uint, n: isize, _allocator: *mut c_void) -> Sexp {
    ret(alloc_vector(t, n))
}

#[inline]
pub unsafe fn alloc_vector(t: c_uint, n: isize) -> Res<Sexp> {
    if n < 0 {
        return Err(error_str("negative length vectors are not allowed"));
    }
    let t = t as u8;
    match t {
        NILSXP => Ok(R_NilValue),
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | EXPRSXP | RAWSXP => try_alloc_vector(t, n as usize),
        CHARSXP => Err(error_str("use of allocVector(CHARSXP ...) is defunct\n")),
        LISTSXP => Ok(Rf_allocList(n as c_int)),
        LANGSXP => {
            if n == 0 {
                return Ok(R_NilValue);
            }
            let s = Rf_allocList(n as c_int);
            s.set_typ(LANGSXP);
            Ok(s)
        }
        _ => Err(error_str(&format!("invalid type/length ({}/{}) in vector allocation", type_to_char_t(t), n))),
    }
}

/// A vector of a caller-chosen length: GNU R's error when there is no
/// memory for it (the core's `alloc_vector` panics, which aborts in
/// `extern "C"`). Small vectors always have a place in a block.
#[inline]
pub unsafe fn try_alloc_vector(t: u8, n: usize) -> Res<Sexp> {
    if n <= 256 {
        return Ok(c().alloc_vector(t, n));
    }
    match c().try_alloc_vector(t, n) {
        Some(x) => Ok(x),
        None => Err(cannot_allocate(t, n as isize)),
    }
}

// R: memory.c:allocVector3 — the size in GNU R's VECREC (8-byte) units,
// reported against no call
#[cold]
unsafe fn cannot_allocate(t: u8, n: isize) -> crate::errors::Unwind {
    let bytes = n as f64 * robigo_core::elt_size(t) as f64;
    let dsize = (bytes / 8.0).ceil() * 8.0 / 1024.0;
    let msg = if dsize > 1024.0 * 1024.0 {
        format!("cannot allocate vector of size {:.1} Gb", dsize / 1024.0 / 1024.0)
    } else if dsize > 1024.0 {
        format!("cannot allocate vector of size {:.1} Mb", dsize / 1024.0)
    } else {
        format!("cannot allocate vector of size {:.0} Kb", dsize)
    };
    crate::errors::errorcall_str(R_NilValue, &msg)
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
    if root_args() {
        return rooted(&[car, cdr], || c().alloc_node(LISTSXP, car, cdr, R_NilValue));
    }
    c().alloc_node(LISTSXP, car, cdr, R_NilValue)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_lcons(car: Sexp, cdr: Sexp) -> Sexp {
    if root_args() {
        return rooted(&[car, cdr], || c().alloc_node(LANGSXP, car, cdr, R_NilValue));
    }
    c().alloc_node(LANGSXP, car, cdr, R_NilValue)
}

/// Experiment A (docs/g3.md §3 item 6, §15.1): `ROBIGO_CAPI_ROOT_ARGS=1`
/// makes the exports the premature-finalization probe found unrooted
/// (`Rf_cons`, `Rf_lcons`, `Rf_setAttrib`) protect their arguments for
/// the whole call, as G3's exports will.
pub(crate) fn root_args() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ROBIGO_CAPI_ROOT_ARGS").is_some_and(|v| v == "1"))
}

/// `f()` with `args` in the precious set, which cannot fail (the protect
/// stack can, and these exports must not jump: docs/core.md §5).
///
/// # Safety
/// `args` are live objects; on the R thread.
pub(crate) unsafe fn rooted<T>(args: &[Sexp], f: impl FnOnce() -> T) -> T {
    for &a in args {
        c().preserve(a);
    }
    let r = f();
    for &a in args.iter().rev() {
        c().release(a);
    }
    r
}

// R: array.c:allocMatrix
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocMatrix(t: c_uint, nrow: c_int, ncol: c_int) -> Sexp {
    ret(alloc_matrix(t, nrow, ncol))
}

pub unsafe fn alloc_matrix(t: c_uint, nrow: c_int, ncol: c_int) -> Res<Sexp> {
    if nrow < 0 || ncol < 0 {
        return Err(error_str("negative extents to matrix"));
    }
    let s = alloc_vector(t, nrow as isize * ncol as isize)?;
    protect(s)?;
    let d = c().alloc_vector(INTSXP, 2);
    *(d.std_data() as *mut i32) = nrow;
    *(d.std_data() as *mut i32).add(1) = ncol;
    crate::attrib::set_attrib(s, R_DimSymbol, d)?;
    unprotect(1)?;
    Ok(s)
}

// R: array.c:alloc3DArray
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_alloc3DArray(t: c_uint, nrow: c_int, ncol: c_int, nface: c_int) -> Sexp {
    ret(alloc_3d_array(t, nrow, ncol, nface))
}

unsafe fn alloc_3d_array(t: c_uint, nrow: c_int, ncol: c_int, nface: c_int) -> Res<Sexp> {
    if nrow < 0 || ncol < 0 || nface < 0 {
        return Err(error_str("negative extents to 3D array"));
    }
    // R: array.c:alloc3DArray checks this against INT_MAX without long
    // vector support; with it, the product must at least exist
    let n = (nrow as isize).checked_mul(ncol as isize).and_then(|n| n.checked_mul(nface as isize));
    let Some(n) = n else { return Err(error_str("'alloc3DArray': too many elements specified")) };
    let s = alloc_vector(t, n)?;
    protect(s)?;
    let d = c().alloc_vector(INTSXP, 3);
    let p = d.std_data() as *mut i32;
    *p = nrow;
    *p.add(1) = ncol;
    *p.add(2) = nface;
    crate::attrib::set_attrib(s, R_DimSymbol, d)?;
    unprotect(1)?;
    Ok(s)
}

// R: array.c:allocArray
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocArray(t: c_uint, dims: Sexp) -> Sexp {
    ret(alloc_array(t, dims))
}

pub unsafe fn alloc_array(t: c_uint, dims: Sexp) -> Res<Sexp> {
    let nd = length(dims)? as usize;
    let p = integer(dims)?;
    let mut n: isize = 1;
    for i in 0..nd {
        // R: array.c:allocArray, as alloc3DArray above
        let Some(m) = n.checked_mul(*p.add(i) as isize) else {
            return Err(error_str("'allocArray': too many elements specified by 'dims'"));
        };
        n = m;
    }
    let s = alloc_vector(t, n)?;
    protect(s)?;
    crate::attrib::set_attrib(s, R_DimSymbol, duplicate(dims)?)?;
    unprotect(1)?;
    Ok(s)
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

/// PROTECT, failing with R_signal_protect_error's error on overflow.
#[inline(always)]
pub unsafe fn protect(s: Sexp) -> Res<Sexp> {
    if R_PPStackTop >= R_PPStackSize {
        return Err(crate::errors::protect_error());
    }
    *R_PPStack.add(R_PPStackTop as usize) = s;
    R_PPStackTop += 1;
    Ok(s)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_unprotect(n: c_int) {
    if R_PPStackTop >= n {
        R_PPStackTop -= n;
    } else {
        crate::errors::R_signal_unprotect_error()
    }
}

/// UNPROTECT, failing with R_signal_unprotect_error's error.
#[inline(always)]
pub unsafe fn unprotect(n: c_int) -> Res<()> {
    if R_PPStackTop >= n {
        R_PPStackTop -= n;
        Ok(())
    } else {
        Err(crate::errors::unprotect_error())
    }
}

// R: memory.c:unprotect_ptr
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_unprotect_ptr(s: Sexp) {
    ret(unprotect_ptr(s))
}

unsafe fn unprotect_ptr(s: Sexp) -> Res<()> {
    let mut i = R_PPStackTop;
    loop {
        if i == 0 {
            return Err(error_str("unprotect_ptr: pointer not found"));
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
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ProtectWithIndex(s: Sexp, pi: *mut c_int) {
    ret(protect(s));
    *pi = R_PPStackTop - 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_Reprotect(s: Sexp, i: c_int) {
    ret(reprotect(s, i))
}

pub unsafe fn reprotect(s: Sexp, i: c_int) -> Res<()> {
    if i >= R_PPStackTop || i < 0 {
        return Err(crate::errors::reprotect_error(i));
    }
    *R_PPStack.add(i as usize) = s;
    Ok(())
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
    ret(new_precious_mset(initial))
}

unsafe fn new_precious_mset(initial: c_int) -> Res<Sexp> {
    let np = c().alloc_vector(INTSXP, 1);
    *(np.std_data() as *mut i32) = 0;
    let mset = protect(Rf_cons(R_NilValue, np))?;
    if initial < 0 {
        return Err(error_str("'initialSize' must be non-negative"));
    }
    let isize = crate::list::Rf_ScalarInteger(initial);
    set_tag(mset, isize)?;
    unprotect(1)?;
    Ok(mset)
}

unsafe fn check_mset(mset: Sexp) -> Res<()> {
    let store = mset.car();
    let np = mset.cdr();
    let isize = mset.tag();
    if (store != R_NilValue && store.typ() != VECSXP)
        || np.typ() != INTSXP
        || np.std_len() != 1
        || isize.typ() != INTSXP
        || isize.std_len() != 1
    {
        return Err(error_str("Invalid mset"));
    }
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_PreserveInMSet(x: Sexp, mset: Sexp) {
    ret(preserve_in_mset(x, mset))
}

unsafe fn preserve_in_mset(x: Sexp, mset: Sexp) -> Res<()> {
    if x == R_NilValue || x.typ() == SYMSXP {
        return Ok(());
    }
    protect(x)?;
    check_mset(mset)?;
    let mut store = mset.car();
    let n = mset.cdr().std_data() as *mut i32;
    if store == R_NilValue {
        let mut newsize = *(mset.tag().std_data() as *const i32) as isize;
        if newsize == 0 {
            newsize = 4;
        }
        store = try_alloc_vector(VECSXP, newsize as usize)?;
        setcar(mset, store)?;
    }
    let size = xlength(store)?;
    if *n as isize == size {
        let newsize = 2 * size;
        if newsize >= i32::MAX as isize {
            return Err(error_str("Multi-set overflow"));
        }
        let ns = protect(try_alloc_vector(VECSXP, newsize as usize)?)?;
        for i in 0..size {
            set_vector_elt(ns, i, vector_elt(store, i)?)?;
        }
        setcar(mset, ns)?;
        unprotect(1)?;
        store = ns;
    }
    unprotect(1)?;
    set_vector_elt(store, *n as isize, x)?;
    *n += 1;
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ReleaseFromMSet(x: Sexp, mset: Sexp) {
    ret(release_from_mset(x, mset))
}

unsafe fn release_from_mset(x: Sexp, mset: Sexp) -> Res<()> {
    if x == R_NilValue || x.typ() == SYMSXP {
        return Ok(());
    }
    check_mset(mset)?;
    let store = mset.car();
    if store == R_NilValue {
        return Ok(());
    }
    let n = mset.cdr().std_data() as *mut i32;
    let mut i = *n as isize - 1;
    while i >= 0 {
        if vector_elt(store, i)? == x {
            while i < *n as isize - 1 {
                set_vector_elt(store, i, vector_elt(store, i + 1)?)?;
                i += 1;
            }
            set_vector_elt(store, i, R_NilValue)?;
            *n -= 1;
            return Ok(());
        }
        i -= 1;
    }
    Ok(())
}

// R: memory.c:R_ReleaseMSet
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ReleaseMSet(mset: Sexp, keep_size: c_int) {
    ret(release_mset(mset, keep_size))
}

unsafe fn release_mset(mset: Sexp, keep_size: c_int) -> Res<()> {
    check_mset(mset)?;
    let store = mset.car();
    if store == R_NilValue {
        return Ok(());
    }
    let n = mset.cdr().std_data() as *mut i32;
    if xlength(store)? <= keep_size as isize {
        for i in 0..*n as isize {
            set_vector_elt(store, i, R_NilValue)?;
        }
    } else {
        setcar(mset, R_NilValue)?;
    }
    *n = 0;
    Ok(())
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
    ret(r_alloc(nelem, eltsize))
}

pub unsafe fn r_alloc(nelem: usize, eltsize: c_int) -> Res<*mut c_char> {
    let size = nelem.wrapping_mul(eltsize as usize);
    let dsize = nelem as f64 * eltsize as f64;
    if dsize > 0.0 {
        if dsize > isize::MAX as f64 {
            return Err(error_str(&format!("cannot allocate memory block of size {:.0} Tb", dsize / 1024f64.powi(4))));
        }
        let s = try_alloc_vector(RAWSXP, size + 1)?;
        s.set_attrib(st().vstack);
        st().vstack = s;
        Ok(s.std_data() as *mut c_char)
    } else {
        Ok(std::ptr::null_mut())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_allocLD(nelem: usize) -> *mut c_void {
    let p = ret(r_alloc(nelem + 1, 16)) as usize;
    ((p + 15) & !15) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn S_alloc(nelem: libc::c_long, eltsize: c_int) -> *mut c_char {
    let size = nelem as usize * eltsize as usize;
    let p = ret(r_alloc(nelem as usize, eltsize));
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
    let q = ret(r_alloc(new as usize, size));
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
        crate::errors::throw(error_str(&format!(
            "'R_Calloc' could not allocate memory ({} of {} bytes)",
            nelem, elsize
        )))
    }
    p
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_chk_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    let p = if ptr.is_null() { libc::malloc(size) } else { libc::realloc(ptr, size) };
    if p.is_null() {
        crate::errors::throw(error_str(&format!("'R_Realloc' could not re-allocate memory ({} bytes)", size)))
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
unsafe fn stack_usage() -> isize {
    let marker = 0u8;
    let sp = std::hint::black_box(&marker) as *const u8 as usize;
    R_CStackDir as isize * (R_CStackStart as isize - sp as isize)
}

// R: errors.c:R_SignalCStackOverflow (the condition is reported without a call)
pub unsafe fn signal_cstack_overflow(usage: isize) -> crate::errors::Unwind {
    crate::errors::errorcall_str(R_NilValue, &format!("C stack usage  {usage} is too close to the limit"))
}

// R: errors.c:R_CheckStack
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_CheckStack() {
    let usage = stack_usage();
    if R_CStackLimit != usize::MAX && usage > R_CStackLimit as isize {
        crate::errors::throw(signal_cstack_overflow(usage))
    }
}

// R: errors.c:R_CheckStack2
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_CheckStack2(extra: usize) {
    let usage = stack_usage();
    if (isize::MAX - usage) < extra as isize {
        crate::errors::throw(signal_cstack_overflow(isize::MAX))
    }
    let usage = usage + extra as isize;
    if R_CStackLimit != usize::MAX && usage > R_CStackLimit as isize {
        crate::errors::throw(signal_cstack_overflow(usage))
    }
}

// ---- header accessors ------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TYPEOF(x: Sexp) -> c_int {
    x.typ() as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_TYPEOF(x: Sexp, t: c_int) {
    watch_write(x);
    x.set_typ(t as u8)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn OBJECT(x: Sexp) -> c_int {
    x.has_flag(flags::OBJECT) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_OBJECT(x: Sexp, v: c_int) {
    crate::bridge::touch_attrs(x);
    watch_write(x);
    x.set_flag(flags::OBJECT, v != 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_S4_OBJECT(x: Sexp) -> c_int {
    (x.gp() & gp::S4 != 0) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_S4_OBJECT(x: Sexp) {
    crate::bridge::touch_attrs(x);
    watch_write(x);
    x.set_gp(x.gp() | gp::S4)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn UNSET_S4_OBJECT(x: Sexp) {
    crate::bridge::touch_attrs(x);
    watch_write(x);
    x.set_gp(x.gp() & !gp::S4)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LEVELS(x: Sexp) -> c_int {
    x.gp() as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SETLEVELS(x: Sexp, v: c_int) -> c_int {
    // the S4 bit is one of the levels
    crate::bridge::touch_attrs(x);
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
    if (v.typ() != LISTSXP && v.typ() != NILSXP) || x.typ() == CHARSXP {
        return ret(set_attrib_list(x, v));
    }
    crate::bridge::touch_attrs(x);
    watch_write(x);
    x.set_attrib(v);
}

/// SET_ATTRIB: `x`'s attributes become the pairlist `v`.
pub unsafe fn set_attrib_list(x: Sexp, v: Sexp) -> Res<()> {
    if v.typ() != LISTSXP && v.typ() != NILSXP {
        return Err(error_str(&format!(
            "value of 'SET_ATTRIB' must be a pairlist or NULL, not a '{}'",
            type_to_char(v)
        )));
    }
    // The slot of a CHARSXP holds its hash (GNU R's holds the cache chain);
    // clearing what ATTRIB reports as NULL changes nothing.
    if x.typ() == CHARSXP {
        if v == R_NilValue {
            return Ok(());
        }
        // R: attrib.c:installAttrib
        return Err(error_str("cannot set attribute on a CHARSXP"));
    }
    crate::bridge::touch_attrs(x);
    watch_write(x);
    x.set_attrib(v);
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CLEAR_ATTRIB(x: Sexp) {
    ret(clear_attrib(x))
}

pub unsafe fn clear_attrib(x: Sexp) -> Res<()> {
    set_attrib_list(x, R_NilValue)?;
    SET_OBJECT(x, 0);
    UNSET_S4_OBJECT(x);
    Ok(())
}

// R: memory.c:DUPLICATE_ATTRIB
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DUPLICATE_ATTRIB(to: Sexp, from: Sexp) {
    ret(duplicate_attrib(to, from, true))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SHALLOW_DUPLICATE_ATTRIB(to: Sexp, from: Sexp) {
    ret(duplicate_attrib(to, from, false))
}

/// DUPLICATE_ATTRIB (`deep`) or SHALLOW_DUPLICATE_ATTRIB.
pub unsafe fn duplicate_attrib(to: Sexp, from: Sexp, deep: bool) -> Res<()> {
    let a = if deep { duplicate(ATTRIB(from))? } else { shallow_duplicate(ATTRIB(from))? };
    set_attrib_list(to, a)?;
    SET_OBJECT(to, OBJECT(from));
    if IS_S4_OBJECT(from) != 0 {
        SET_S4_OBJECT(to)
    } else {
        UNSET_S4_OBJECT(to)
    }
    Ok(())
}

// ---- reference counts ------------------------------------------------------
// (every store keeps them: robigo_core's `Sexp::store`, GNU R's FIX_REFCNT)

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

/// memory.c's `nvec` table: the types LENGTH and friends refuse.
#[inline]
fn nvec(t: u8) -> bool {
    !matches!(t, CHARSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | EXPRSXP | WEAKREFSXP | RAWSXP)
}

/// The length of a vector, ALTREP-aware (a class's Length method can
/// fail).
#[inline]
pub unsafe fn xlength(x: Sexp) -> Res<isize> {
    if x.is_altrep() { crate::altrep::length(x) } else { Ok(x.std_len()) }
}

#[inline(always)]
unsafe fn chk2(x: Sexp) -> Res<()> {
    if nvec(x.typ()) {
        return Err(chk2_error(x));
    }
    Ok(())
}

#[cold]
#[inline(never)]
unsafe fn chk2_error(x: Sexp) -> Unwind {
    error_str(&format!("LENGTH or similar applied to {} object", type_to_char(x)))
}

/// The length of a vector LENGTH and XLENGTH answer without a call: a
/// standard vector's, or one of robigo's own compact sequences'.
#[inline(always)]
unsafe fn plain_xlength(x: Sexp) -> Option<isize> {
    if nvec(x.typ()) {
        None
    } else if !x.is_altrep() {
        Some(x.std_len())
    } else {
        crate::altrep::own_length(x)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LENGTH(x: Sexp) -> c_int {
    if let Some(n) = plain_xlength(x).filter(|&n| n <= i32::MAX as isize) {
        return n as c_int;
    }
    ret(length(x))
}

/// LENGTH: the length of a vector of at most `INT_MAX` elements.
pub unsafe fn length(x: Sexp) -> Res<c_int> {
    if x == R_NilValue {
        return Ok(0);
    }
    chk2(x)?;
    let n = xlength(x)?;
    if n > i32::MAX as isize {
        return Err(crate::errors::bad_long_vector(x, c"memory.c".as_ptr(), 0));
    }
    Ok(n as c_int)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn XLENGTH(x: Sexp) -> isize {
    if let Some(n) = plain_xlength(x) {
        return n;
    }
    ret(checked_xlength(x))
}

/// XLENGTH: [`xlength`] of a vector type.
pub unsafe fn checked_xlength(x: Sexp) -> Res<isize> {
    chk2(x)?;
    xlength(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_XLENGTH(x: Sexp) -> isize {
    ret(checked_xlength(x))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TRUELENGTH(x: Sexp) -> isize {
    ret(truelength(x))
}

/// TRUELENGTH.
pub unsafe fn truelength(x: Sexp) -> Res<isize> {
    chk2(x)?;
    Ok(if x.is_altrep() { 0 } else { x.truelen() })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_TRUELENGTH(x: Sexp, v: isize) {
    ret(set_truelength(x, v))
}

unsafe fn set_truelength(x: Sexp, v: isize) -> Res<()> {
    chk2(x)?;
    // R: Defn.h:SET_TRUELENGTH (the slot is data2 of an ALTREP node)
    if x.is_altrep() {
        return Err(error_str("can't set ALTREP truelength"));
    }
    x.set_truelen(v);
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SETLENGTH(x: Sexp, v: isize) {
    ret(setlength(x, v))
}

unsafe fn setlength(x: Sexp, v: isize) -> Res<()> {
    if x.is_altrep() {
        return Err(error_str("SETLENGTH() cannot be applied to an ALTVEC object."));
    }
    if !crate::util::is_vector(x) {
        return Err(error_str(&format!(
            "SETLENGTH() can only be applied to a standard vector, not a '{}'",
            type_to_char(x)
        )));
    }
    watch_write(x);
    x.set_std_len(v);
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_LONG_VEC(x: Sexp) -> c_int {
    ret(chk2(x));
    (ret(xlength(x)) > i32::MAX as isize) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_GROWABLE(x: Sexp) -> c_int {
    (x.gp() & gp::GROWABLE != 0 && ret(xlength(x)) < x.truelen()) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_GROWABLE_BIT(x: Sexp) {
    x.set_gp(x.gp() | gp::GROWABLE)
}

/// A write to `x` through the API: report it when the bridge watches `x`
/// (robigo_core::extra::TWIN, docs/core.md §6).
#[inline(always)]
pub unsafe fn watch_write(x: Sexp) {
    if x.is_twin_cell() {
        crate::bridge::twin_written(x);
    }
}

/// A read-only payload pointer of `x` handed to C: C writes the cells of a
/// character vector or list through it too, since GNU R's pointer is the
/// object's own (data.table's `setorder` reorders columns with
/// `memcpy((char *)DATAPTR_RO(v) + ..., ...)`, Rcpp's `x.sort()` and
/// cpp11 write through `STRING_PTR_RO`). So a watched one reports a
/// possible write, which the bridge compares on write-back. Atomic
/// vectors need none: a twin's buffer is compared at every call (or is
/// the value's own, zero-copy).
#[inline(always)]
unsafe fn watch_ro_ptr(x: Sexp) {
    if matches!(x.typ(), STRSXP | VECSXP | EXPRSXP) {
        watch_write(x);
    }
}

/// DATAPTR with GNU R's zero-length convention, for writing: the caller
/// may store through the pointer, so a watched object reports a write.
#[inline]
pub unsafe fn dataptr(x: Sexp) -> Res<*mut u8> {
    watch_write(x);
    dataptr_unwatched(x)
}

/// [`dataptr`] for reading (or for the core's own copies).
#[inline]
pub unsafe fn dataptr_unwatched(x: Sexp) -> Res<*mut u8> {
    if x.is_altrep() {
        return crate::altrep::dataptr(x, true);
    }
    Ok(std_dataptr(x))
}

/// [`dataptr_unwatched`] of a vector that is not ALTREP.
#[inline]
pub unsafe fn std_dataptr(x: Sexp) -> *mut u8 {
    debug_assert!(!x.is_altrep());
    if x.std_len() == 0 && x.typ() != CHARSXP {
        return std::ptr::dangling_mut::<u8>();
    }
    x.std_data()
}

/// DATAPTR_RO.
#[inline]
pub unsafe fn dataptr_ro(x: Sexp) -> Res<*const u8> {
    if x.is_altrep() {
        return Ok(crate::altrep::dataptr(x, false)? as *const u8);
    }
    Ok(std_dataptr(x) as *const u8)
}

/// DATAPTR_OR_NULL.
#[inline]
pub unsafe fn dataptr_or_null(x: Sexp) -> Res<*const u8> {
    if x.is_altrep() {
        return Ok(crate::altrep::dataptr_or_null(x)? as *const u8);
    }
    Ok(std_dataptr(x) as *const u8)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DATAPTR(x: Sexp) -> *mut c_void {
    watch_write(x);
    if !x.is_altrep() {
        return std_dataptr(x) as *mut c_void;
    }
    ret(crate::altrep::dataptr(x, true)) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DATAPTR_RO(x: Sexp) -> *const c_void {
    watch_ro_ptr(x);
    if !x.is_altrep() {
        return std_dataptr(x) as *const c_void;
    }
    ret(crate::altrep::dataptr(x, false)) as *const c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DATAPTR_OR_NULL(x: Sexp) -> *const c_void {
    watch_ro_ptr(x);
    ret(dataptr_or_null(x)) as *const c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn STDVEC_DATAPTR(x: Sexp) -> *mut c_void {
    if x.is_altrep() {
        crate::errors::throw(error_str("cannot get STDVEC_DATAPTR from ALTREP object"))
    }
    watch_write(x);
    std_dataptr(x) as *mut c_void
}

macro_rules! typed_ptr {
    ($name:ident, $ro:ident, $ornull:ident, $inner:ident, $inner_ro:ident, $t:ty, $fname:expr, $want:expr, $($types:pat_param)|+) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(x: Sexp) -> *mut $t {
            if matches!(x.typ(), $($types)|+) {
                watch_write(x);
                if !x.is_altrep() {
                    return std_dataptr(x) as *mut $t;
                }
                if let Some(p) = crate::altrep::own_expanded_data(x) {
                    return p as *mut $t;
                }
            }
            ret($inner(x))
        }
        #[doc = concat!("`", stringify!($name), "()`.")]
        #[inline]
        pub unsafe fn $inner(x: Sexp) -> Res<*mut $t> {
            if !matches!(x.typ(), $($types)|+) {
                return Err(wrong_type($fname, $want, x));
            }
            Ok(dataptr(x)? as *mut $t)
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $ro(x: Sexp) -> *const $t {
            if matches!(x.typ(), $($types)|+) {
                if !x.is_altrep() {
                    return std_dataptr(x) as *const $t;
                }
                if let Some(p) = crate::altrep::own_expanded_data(x) {
                    return p as *const $t;
                }
            }
            ret($inner_ro(x))
        }
        #[doc = concat!("`", stringify!($ro), "()`.")]
        #[inline]
        pub unsafe fn $inner_ro(x: Sexp) -> Res<*const $t> {
            if !matches!(x.typ(), $($types)|+) {
                return Err(wrong_type($fname, $want, x));
            }
            Ok(dataptr_ro(x)? as *const $t)
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $ornull(x: Sexp) -> *const $t {
            if !matches!(x.typ(), $($types)|+) {
                crate::errors::throw(wrong_type($fname, $want, x))
            }
            ret(dataptr_or_null(x)) as *const $t
        }
    };
}

typed_ptr!(LOGICAL, LOGICAL_RO, LOGICAL_OR_NULL, logical, logical_ro, c_int, "LOGICAL", "logical", LGLSXP);
typed_ptr!(INTEGER, INTEGER_RO, INTEGER_OR_NULL, integer, integer_ro, c_int, "INTEGER", "integer", INTSXP | LGLSXP);
typed_ptr!(REAL, REAL_RO, REAL_OR_NULL, real, real_ro, f64, "REAL", "numeric", REALSXP);
typed_ptr!(COMPLEX, COMPLEX_RO, COMPLEX_OR_NULL, complex, complex_ro, Rcomplex, "COMPLEX", "complex", CPLXSXP);
typed_ptr!(RAW, RAW_RO, RAW_OR_NULL, raw, raw_ro, u8, "RAW", "raw", RAWSXP);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn STRING_PTR(x: Sexp) -> *mut Sexp {
    if x.typ() != STRSXP {
        crate::errors::throw(wrong_type("STRING_PTR", "character", x))
    }
    ret(dataptr(x)) as *mut Sexp
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn STRING_PTR_RO(x: Sexp) -> *const Sexp {
    watch_ro_ptr(x);
    ret(string_ptr_ro(x))
}

/// STRING_PTR_RO.
pub unsafe fn string_ptr_ro(x: Sexp) -> Res<*const Sexp> {
    if x.typ() != STRSXP {
        return Err(wrong_type("STRING_PTR_RO", "character", x));
    }
    Ok(dataptr_ro(x)? as *const Sexp)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn VECTOR_PTR_RO(x: Sexp) -> *const Sexp {
    if x.typ() != VECSXP {
        crate::errors::throw(wrong_type("VECTOR_PTR_RO", "list", x))
    }
    watch_ro_ptr(x);
    ret(dataptr_ro(x)) as *const Sexp
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn VECTOR_PTR(_x: Sexp) -> *mut Sexp {
    crate::errors::throw(error_str("not safe to return vector pointer"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_CHAR(x: Sexp) -> *const c_char {
    ret(r_char(x))
}

/// CHAR.
pub unsafe fn r_char(x: Sexp) -> Res<*const c_char> {
    if x.typ() != CHARSXP {
        return Err(wrong_type("CHAR", "CHARSXP", x));
    }
    Ok(x.std_data() as *const c_char)
}

// R: memory.c:STRING_ELT
#[unsafe(no_mangle)]
pub unsafe extern "C" fn STRING_ELT(x: Sexp, i: isize) -> Sexp {
    if x.typ() == STRSXP && !x.is_altrep() {
        return *(x.std_data() as *const Sexp).offset(i);
    }
    ret(string_elt_slow(x, i))
}

/// STRING_ELT.
#[inline]
pub unsafe fn string_elt(x: Sexp, i: isize) -> Res<Sexp> {
    if x.typ() == STRSXP && !x.is_altrep() {
        return Ok(*(x.std_data() as *const Sexp).offset(i));
    }
    string_elt_slow(x, i)
}

#[cold]
#[inline(never)]
unsafe fn string_elt_slow(x: Sexp, i: isize) -> Res<Sexp> {
    if x.typ() != STRSXP {
        return Err(wrong_type("STRING_ELT", "character vector", x));
    }
    if x.is_altrep() {
        return crate::altrep::string_elt(x, i);
    }
    Ok(*(x.std_data() as *const Sexp).offset(i))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn VECTOR_ELT(x: Sexp, i: isize) -> Sexp {
    if matches!(x.typ(), VECSXP | EXPRSXP) && !x.is_altrep() {
        return *(x.std_data() as *const Sexp).offset(i);
    }
    ret(vector_elt_slow(x, i))
}

/// VECTOR_ELT.
#[inline]
pub unsafe fn vector_elt(x: Sexp, i: isize) -> Res<Sexp> {
    if matches!(x.typ(), VECSXP | EXPRSXP) && !x.is_altrep() {
        return Ok(*(x.std_data() as *const Sexp).offset(i));
    }
    vector_elt_slow(x, i)
}

#[cold]
#[inline(never)]
unsafe fn vector_elt_slow(x: Sexp, i: isize) -> Res<Sexp> {
    if !matches!(x.typ(), VECSXP | EXPRSXP | WEAKREFSXP) {
        return Err(wrong_type("VECTOR_ELT", "list", x));
    }
    if x.is_altrep() {
        let a = crate::altrep::list_elt(x, i)?;
        MARK_NOT_MUTABLE(a);
        return Ok(a);
    }
    Ok(*(x.std_data() as *const Sexp).offset(i))
}

#[inline(always)]
unsafe fn plain_target(x: Sexp, i: isize) -> bool {
    // the flags byte (+1) and the extra word (+6) in one header load: not
    // ALTREP, not waiting for the store barrier (an immortal's first
    // store), not watched; `set_elt` repeats the barrier's test on the
    // same load, which the compiler drops
    const MASK: u64 = ((flags::ALTREP | flags::BARRIER) as u64) << 8 | (robigo_core::extra::TWIN as u64) << 48;
    (*(x.0 as *const u64) & MASK) == 0 && (i as usize) < x.std_len() as usize
}

// R: memory.c:SET_STRING_ELT
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_STRING_ELT(x: Sexp, i: isize, v: Sexp) {
    if x.typ() == STRSXP && v.typ() == CHARSXP && plain_target(x, i) {
        return x.set_elt(i as usize, v);
    }
    ret(set_string_elt_slow(x, i, v))
}

/// SET_STRING_ELT.
#[inline]
pub unsafe fn set_string_elt(x: Sexp, i: isize, v: Sexp) -> Res<()> {
    if x.typ() == STRSXP && v.typ() == CHARSXP && plain_target(x, i) {
        x.set_elt(i as usize, v);
        return Ok(());
    }
    set_string_elt_slow(x, i, v)
}

#[cold]
#[inline(never)]
unsafe fn set_string_elt_slow(x: Sexp, i: isize, v: Sexp) -> Res<()> {
    if x.typ() != STRSXP {
        return Err(wrong_type("SET_STRING_ELT", "character vector", x));
    }
    if v.typ() != CHARSXP {
        return Err(error_str(&format!("Value of SET_STRING_ELT() must be a 'CHARSXP' not a '{}'", type_to_char(v))));
    }
    let n = xlength(x)?;
    if i < 0 || i >= n {
        return Err(error_str(&format!("attempt to set index {i}/{n} in SET_STRING_ELT")));
    }
    if x.is_altrep() {
        return crate::altrep::string_set_elt(x, i, v);
    }
    watch_write(x);
    x.set_elt(i as usize, v);
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_VECTOR_ELT(x: Sexp, i: isize, v: Sexp) -> Sexp {
    if matches!(x.typ(), VECSXP | EXPRSXP) && plain_target(x, i) {
        x.set_elt(i as usize, v);
        return v;
    }
    ret(set_vector_elt_slow(x, i, v))
}

/// SET_VECTOR_ELT.
#[inline]
pub unsafe fn set_vector_elt(x: Sexp, i: isize, v: Sexp) -> Res<Sexp> {
    if matches!(x.typ(), VECSXP | EXPRSXP) && plain_target(x, i) {
        x.set_elt(i as usize, v);
        return Ok(v);
    }
    set_vector_elt_slow(x, i, v)
}

#[cold]
#[inline(never)]
unsafe fn set_vector_elt_slow(x: Sexp, i: isize, v: Sexp) -> Res<Sexp> {
    if !matches!(x.typ(), VECSXP | EXPRSXP | WEAKREFSXP) {
        return Err(wrong_type("SET_VECTOR_ELT", "list", x));
    }
    let n = xlength(x)?;
    if i < 0 || i >= n {
        return Err(error_str(&format!("attempt to set index {i}/{n} in SET_VECTOR_ELT")));
    }
    if x.is_altrep() {
        crate::altrep::list_set_elt(x, i, v)?;
        return Ok(v);
    }
    watch_write(x);
    x.set_elt(i as usize, v);
    Ok(v)
}

macro_rules! elt_accessors {
    ($get:ident, $set:ident, $inner_get:ident, $ptr:ident, $t:ty, $elt:ident, $($types:pat_param)|+) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $get(x: Sexp, i: isize) -> $t {
            if matches!(x.typ(), $($types)|+) && !x.is_altrep() {
                return *(std_dataptr(x) as *const $t).offset(i);
            }
            ret($inner_get(x, i))
        }
        #[doc = concat!("`", stringify!($get), "()`.")]
        #[inline]
        pub unsafe fn $inner_get(x: Sexp, i: isize) -> Res<$t> {
            if x.is_altrep() {
                return crate::altrep::$elt(x, i);
            }
            if !matches!(x.typ(), $($types)|+) {
                $ptr(x)?;
            }
            Ok(*(std_dataptr(x) as *const $t).offset(i))
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $set(x: Sexp, i: isize, v: $t) {
            *ret($ptr(x)).offset(i) = v;
        }
    };
}

elt_accessors!(INTEGER_ELT, SET_INTEGER_ELT, integer_elt, integer, c_int, integer_elt, INTSXP | LGLSXP);
elt_accessors!(LOGICAL_ELT, SET_LOGICAL_ELT, logical_elt, logical, c_int, logical_elt, LGLSXP);
elt_accessors!(REAL_ELT, SET_REAL_ELT, real_elt, real, f64, real_elt, REALSXP);
elt_accessors!(COMPLEX_ELT, SET_COMPLEX_ELT, complex_elt, complex, Rcomplex, complex_elt, CPLXSXP);
elt_accessors!(RAW_ELT, SET_RAW_ELT, raw_elt, raw, u8, raw_elt, RAWSXP);

// R: altrep.c:INTEGER_GET_REGION & co — the class's Get_region method,
// else element by element (no materialization)
macro_rules! get_region {
    ($name:ident, $t:ty, $elt:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(x: Sexp, i: isize, n: isize, buf: *mut $t) -> isize {
            ret(get_region::<$t>(x, i, n, buf as *mut u8, |x, i| $elt(x, i)))
        }
    };
}

/// INTEGER_GET_REGION & co: `n` elements from `i` into `buf`, by the
/// class's Get_region method or element by element.
#[inline]
unsafe fn get_region<T>(x: Sexp, i: isize, n: isize, buf: *mut u8, elt: impl Fn(Sexp, isize) -> Res<T>) -> Res<isize> {
    if x.is_altrep() {
        let f = (*(x.tag().std_data() as *const crate::altrep::Methods)).get_region;
        if f != 0 {
            let r = crate::errors::call_w6(f, [x.0 as usize, i as usize, n as usize, buf as usize, 0, 0])?;
            return Ok(r as isize);
        }
    }
    let len = xlength(x)?;
    let ncopy = if len - i > n { n } else { len - i };
    let buf = buf as *mut T;
    for k in 0..ncopy.max(0) {
        *buf.offset(k) = elt(x, i + k)?;
    }
    Ok(ncopy)
}

get_region!(INTEGER_GET_REGION, c_int, integer_elt);
get_region!(LOGICAL_GET_REGION, c_int, logical_elt);
get_region!(REAL_GET_REGION, f64, real_elt);
get_region!(COMPLEX_GET_REGION, Rcomplex, complex_elt);
get_region!(RAW_GET_REGION, u8, raw_elt);

// ---- nodes ----------------------------------------------------------------

/// Field `k` (0 car, 1 cdr, 2 tag) of a node; HOST proxies (robigo
/// environments, closures, promises before G3) ask the host.
#[inline]
pub unsafe fn field(x: Sexp, k: c_int) -> Res<Sexp> {
    if x.has_flag(flags::HOST) {
        let ks = c().alloc_vector(INTSXP, 1);
        *(ks.std_data() as *mut i32) = k;
        return crate::host::op("field\0", &[x, ks]);
    }
    if x.typ() == PROMSXP {
        crate::bridge::sync_stale_promise(x)?;
    }
    Ok(match k {
        0 => x.car(),
        1 => x.cdr(),
        _ => x.tag(),
    })
}

macro_rules! node_get {
    ($($name:ident = $($step:literal),+;)*) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(x: Sexp) -> Sexp {
                let mut v = x;
                $(v = ret(field(v, $step));)+
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
    PRVALUE = 0;
    PRCODE = 1;
    PRENV = 2;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CAR0(x: Sexp) -> Sexp {
    x.car()
}

/// R's CHKCONS for the setters: the node must be cons-shaped.
unsafe fn chkcons(x: Sexp) -> Res<()> {
    if x == R_NilValue || x.is_null() {
        return Err(error_str("bad value"));
    }
    Ok(())
}

unsafe fn set_field(x: Sexp, k: c_int, y: Sexp) -> Res<()> {
    let ks = c().alloc_vector(INTSXP, 1);
    *(ks.std_data() as *mut i32) = k;
    crate::host::op("set_field\0", &[x, ks, y])?;
    Ok(())
}

/// The cons setters' exports, written out as GNU R's (an error raised
/// in place, the `Res` steps through `ret`): the hot path costs what a
/// store does. Rust code calls [`setcar`], [`setcdr`], [`set_tag`].
macro_rules! node_setter_exports {
    ($($name:ident = $store:ident, $k:literal;)*) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(x: Sexp, y: Sexp) -> Sexp {
                if x == R_NilValue || x.is_null() {
                    crate::errors::throw(bad_value())
                }
                if x.has_flag(flags::HOST) {
                    ret(set_field(x, $k, y));
                    return y;
                }
                if x.extra() & robigo_core::extra::WATCHED != 0 {
                    crate::bridge::watched_cell_written(x);
                }
                watch_write(x);
                x.$store(y);
                if x.typ() == PROMSXP {
                    ret(crate::bridge::sync_promise_from_node(x));
                }
                y
            }
        )*
    };
}

node_setter_exports! {
    SETCAR = set_car, 0;
    SETCDR = set_cdr, 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_TAG(x: Sexp, y: Sexp) {
    if x == R_NilValue || x.is_null() {
        crate::errors::throw(bad_value())
    }
    if x.has_flag(flags::HOST) {
        return ret(set_field(x, 2, y));
    }
    if x.extra() & robigo_core::extra::WATCHED != 0 {
        crate::bridge::watched_cell_written(x);
    }
    watch_write(x);
    x.set_tag(y);
    if x.typ() == PROMSXP {
        ret(crate::bridge::sync_promise_from_node(x));
    }
}

#[cold]
unsafe fn bad_value() -> crate::errors::Unwind {
    error_str("bad value")
}

/// SETCAR.
pub unsafe fn setcar(x: Sexp, y: Sexp) -> Res<Sexp> {
    chkcons(x)?;
    if x.has_flag(flags::HOST) {
        set_field(x, 0, y)?;
        return Ok(y);
    }
    if x.extra() & robigo_core::extra::WATCHED != 0 {
        crate::bridge::watched_cell_written(x);
    }
    watch_write(x);
    x.set_car(y);
    // A promise node written through the cons setter: keep the
    // memoized runtime promise in step (GNU R defines SET_PRVALUE as
    // SETCAR; rlang writes PRCODE/PRENV through CDR/TAG the same way).
    if x.typ() == PROMSXP {
        crate::bridge::sync_promise_from_node(x)?;
    }
    Ok(y)
}

/// SETCDR.
pub unsafe fn setcdr(x: Sexp, y: Sexp) -> Res<Sexp> {
    chkcons(x)?;
    if x.has_flag(flags::HOST) {
        set_field(x, 1, y)?;
        return Ok(y);
    }
    if x.extra() & robigo_core::extra::WATCHED != 0 {
        crate::bridge::watched_cell_written(x);
    }
    watch_write(x);
    x.set_cdr(y);
    if x.typ() == PROMSXP {
        crate::bridge::sync_promise_from_node(x)?;
    }
    Ok(y)
}

/// SET_TAG.
pub unsafe fn set_tag(x: Sexp, y: Sexp) -> Res<()> {
    chkcons(x)?;
    if x.has_flag(flags::HOST) {
        return set_field(x, 2, y);
    }
    if x.extra() & robigo_core::extra::WATCHED != 0 {
        crate::bridge::watched_cell_written(x);
    }
    watch_write(x);
    x.set_tag(y);
    if x.typ() == PROMSXP {
        crate::bridge::sync_promise_from_node(x)?;
    }
    Ok(())
}

macro_rules! setcxr {
    ($($name:ident = $($step:ident).*;)*) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(x: Sexp, y: Sexp) -> Sexp {
                let cell = x$(.$step())*;
                ret(setcar(cell, y))
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
    ret(setcar(x, v));
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_BODY(x: Sexp, v: Sexp) {
    ret(setcdr(x, v));
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_CLOENV(x: Sexp, v: Sexp) {
    ret(set_tag(x, v))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_PRVALUE(x: Sexp, v: Sexp) {
    ret(setcar(x, v));
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_PRCODE(x: Sexp, v: Sexp) {
    ret(setcdr(x, v));
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SET_PRENV(x: Sexp, v: Sexp) {
    ret(set_tag(x, v))
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
    ret(r_length(s))
}

/// Rinlinedfuns.h's `length()`: of any object.
pub unsafe fn r_length(s: Sexp) -> Res<c_int> {
    Ok(match s.typ() {
        NILSXP => 0,
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | CHARSXP | VECSXP | EXPRSXP | RAWSXP => length(s)?,
        LISTSXP | LANGSXP | DOTSXP => {
            let mut i = 0;
            let mut p = s;
            while !p.is_null() && p != R_NilValue {
                i += 1;
                p = p.cdr();
            }
            i
        }
        ENVSXP => crate::util::env_length(s)?,
        _ => 1,
    })
}

// R: Rinlinedfuns.h:xlength
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_xlength(s: Sexp) -> isize {
    ret(r_xlength(s))
}

/// Rinlinedfuns.h's `xlength()`: of any object.
pub unsafe fn r_xlength(s: Sexp) -> Res<isize> {
    Ok(match s.typ() {
        NILSXP => 0,
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | CHARSXP | VECSXP | EXPRSXP | RAWSXP => xlength(s)?,
        LISTSXP | LANGSXP | DOTSXP => r_length(s)? as isize,
        ENVSXP => crate::util::env_xlength(s)?,
        _ => 1,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LENGTH_EX(x: Sexp, _file: *const c_char, _line: c_int) -> c_int {
    ret(length(x))
}

// ---- duplication (duplicate.c) -------------------------------------------------

unsafe fn duplicate1(s: Sexp, deep: bool) -> Res<Sexp> {
    let nil = R_NilValue;
    let t = s.typ();
    match t {
        NILSXP | SYMSXP | ENVSXP | SPECIALSXP | BUILTINSXP | EXTPTRSXP | BCODESXP | WEAKREFSXP | CHARSXP | PROMSXP => {
            return Ok(s);
        }
        _ => {}
    }
    if s.has_flag(flags::HOST) {
        return Ok(s);
    }
    let dup_child = |x: Sexp| if deep { duplicate1(x, true) } else { Ok(x) };
    let r = match t {
        CLOSXP => c().alloc_node(CLOSXP, s.car(), s.cdr(), s.tag()),
        LISTSXP | LANGSXP | DOTSXP => {
            // duplicate the spine (and the cars when deep)
            protect(s)?;
            let head = c().alloc_node(t, nil, nil, nil);
            protect(head)?;
            let mut src = s;
            let mut dst = head;
            loop {
                let car = dup_child(src.car())?;
                setcar(dst, car)?;
                set_tag(dst, src.tag())?;
                dst.set_gp(src.gp());
                let a = src.attrib_raw();
                if a != nil {
                    set_attrib_list(dst, duplicate1(a, deep)?)?;
                }
                dst.set_flags((dst.flags() & !flags::OBJECT) | (src.flags() & flags::OBJECT));
                src = src.cdr();
                if src == nil || src.typ() != t && !matches!(src.typ(), LISTSXP | LANGSXP | DOTSXP) {
                    if src != nil {
                        setcdr(dst, dup_child(src)?)?;
                    }
                    break;
                }
                let next = c().alloc_node(LISTSXP, nil, nil, nil);
                setcdr(dst, next)?;
                dst = next;
            }
            unprotect(2)?;
            return Ok(head);
        }
        LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP | STRSXP | VECSXP | EXPRSXP => {
            let n = xlength(s)?;
            protect(s)?;
            let r = try_alloc_vector(t, n as usize)?;
            if n > 0 {
                let src = dataptr_unwatched(s)? as *const u8;
                match t {
                    STRSXP => {
                        let d = src as *const Sexp;
                        for i in 0..n as usize {
                            r.set_elt(i, *d.add(i));
                        }
                    }
                    VECSXP | EXPRSXP => {
                        protect(r)?;
                        for i in 0..n {
                            let e = vector_elt(s, i)?;
                            set_vector_elt(r, i, dup_child(e)?)?;
                        }
                        unprotect(1)?;
                    }
                    _ => std::ptr::copy_nonoverlapping(src, r.std_data(), n as usize * robigo_core::elt_size(t)),
                }
            }
            unprotect(1)?;
            // a duplicate is never growable
            r
        }
        OBJSXP => c().alloc_node(OBJSXP, nil, nil, nil),
        _ => return Err(error_str("unimplemented type in duplicate")),
    };
    protect(r)?;
    let a = s.attrib_raw();
    if a != nil {
        set_attrib_list(r, duplicate1(a, deep)?)?;
    }
    r.set_flags((r.flags() & !flags::OBJECT) | (s.flags() & flags::OBJECT));
    r.set_gp((r.gp() & !gp::S4) | (s.gp() & gp::S4));
    unprotect(1)?;
    Ok(r)
}

// R: duplicate.c:duplicate
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_duplicate(s: Sexp) -> Sexp {
    ret(duplicate(s))
}

/// duplicate.
pub unsafe fn duplicate(s: Sexp) -> Res<Sexp> {
    if s.is_altrep() {
        return crate::altrep::duplicate(s, true);
    }
    duplicate1(s, true)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_shallow_duplicate(s: Sexp) -> Sexp {
    ret(shallow_duplicate(s))
}

/// shallow_duplicate.
pub unsafe fn shallow_duplicate(s: Sexp) -> Res<Sexp> {
    if s.is_altrep() {
        return crate::altrep::duplicate(s, false);
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
    ret(copy_vector(s, t))
}

/// Element `k` of `t` into slot `i` of `s` (same type).
unsafe fn copy_elt(s: Sexp, i: isize, t: Sexp, k: isize, what: &str) -> Res<()> {
    match s.typ() {
        STRSXP => set_string_elt(s, i, string_elt(t, k)?)?,
        VECSXP | EXPRSXP => {
            set_vector_elt(s, i, Rf_lazy_duplicate(vector_elt(t, k)?))?;
        }
        LGLSXP | INTSXP => *integer(s)?.offset(i) = *integer(t)?.offset(k),
        REALSXP => *real(s)?.offset(i) = *real(t)?.offset(k),
        CPLXSXP => *complex(s)?.offset(i) = *complex(t)?.offset(k),
        RAWSXP => *raw(s)?.offset(i) = *raw(t)?.offset(k),
        _ => return Err(error_str(&format!("unimplemented type in {what}"))),
    }
    Ok(())
}

/// copyVector.
pub unsafe fn copy_vector(s: Sexp, t: Sexp) -> Res<()> {
    if s.typ() != t.typ() {
        return Err(error_str("vector types do not match in copyVector"));
    }
    let ns = xlength(s)?;
    let nt = xlength(t)?;
    if nt == 0 {
        return Ok(());
    }
    for i in 0..ns {
        copy_elt(s, i, t, i % nt, "copyVector")?;
    }
    Ok(())
}

// R: duplicate.c:copyMatrix
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_copyMatrix(s: Sexp, t: Sexp, byrow: c_int) {
    ret(copy_matrix(s, t, byrow))
}

unsafe fn copy_matrix(s: Sexp, t: Sexp, byrow: c_int) -> Res<()> {
    if byrow == 0 {
        return copy_vector(s, t);
    }
    let nr = crate::util::nrows(s)? as isize;
    let nc = crate::util::ncols(s)? as isize;
    let nt = xlength(t)?;
    let mut k = 0isize;
    for i in 0..nr {
        for j in 0..nc {
            copy_elt(s, i + j * nr, t, k % nt, "copyMatrix")?;
            k += 1;
        }
    }
    Ok(())
}
