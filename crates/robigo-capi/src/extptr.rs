//! External pointers, weak references and finalizers (memory.c).

use crate::errors::error_str;
use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Sexp, gp};
use std::ffi::{c_int, c_void};

// R: memory.c:R_MakeExternalPtr
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_MakeExternalPtr(p: *mut c_void, tag: Sexp, prot: Sexp) -> Sexp {
    core().alloc_node(EXTPTRSXP, Sexp(p as *mut u8), prot, tag)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ExternalPtrAddr(s: Sexp) -> *mut c_void {
    s.car().0 as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ExternalPtrTag(s: Sexp) -> Sexp {
    s.tag()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ExternalPtrProtected(s: Sexp) -> Sexp {
    s.cdr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ClearExternalPtr(s: Sexp) {
    s.set_car_raw(Sexp::NULL)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_SetExternalPtrAddr(s: Sexp, p: *mut c_void) {
    s.set_car_raw(Sexp(p as *mut u8))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_SetExternalPtrTag(s: Sexp, tag: Sexp) {
    SET_TAG(s, tag)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_SetExternalPtrProtected(s: Sexp, p: Sexp) {
    SETCDR(s, p);
}

// R: memory.c:R_MakeExternalPtrFn — the function pointer in the address slot
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_MakeExternalPtrFn(p: *const c_void, tag: Sexp, prot: Sexp) -> Sexp {
    R_MakeExternalPtr(p as *mut c_void, tag, prot)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_ExternalPtrAddrFn(s: Sexp) -> *const c_void {
    s.car().0 as *const c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EXTPTR_PTR(s: Sexp) -> *mut c_void {
    s.car().0 as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EXTPTR_TAG(s: Sexp) -> Sexp {
    s.tag()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EXTPTR_PROT(s: Sexp) -> Sexp {
    s.cdr()
}

// ---- weak references --------------------------------------------------

unsafe fn weak_field(w: Sexp, i: usize) -> Sexp {
    *(w.std_data() as *const Sexp).add(i)
}

unsafe fn set_weak_field(w: Sexp, i: usize, v: Sexp) {
    *(w.std_data() as *mut Sexp).add(i) = v;
}

// R: memory.c:NewWeakRef
unsafe fn new_weak_ref(key: Sexp, val: Sexp, fin: Sexp, onexit: bool) -> Sexp {
    match key.typ() {
        NILSXP | ENVSXP | EXTPTRSXP | BCODESXP => {}
        _ => error_str("can only weakly reference/finalize reference objects"),
    }
    Rf_protect(key);
    Rf_protect(val);
    Rf_protect(fin);
    let nil = R_NilValue;
    let w = core().alloc_vector(WEAKREFSXP, 4);
    if key != nil {
        // a host proxy key would die while the host still holds the
        // object: keep it alive instead (never finalized)
        if key.has_flag(robigo_core::flags::HOST) {
            core().preserve(key);
        }
        set_weak_field(w, 0, key);
        set_weak_field(w, 1, val);
        set_weak_field(w, 2, fin);
        if onexit {
            w.set_gp(w.gp() | gp::FINALIZE_ON_EXIT);
        }
        core().register_weak_ref(w);
    }
    let _ = (nil, Rf_unprotect(3));
    w
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_MakeWeakRef(key: Sexp, val: Sexp, fin: Sexp, onexit: c_int) -> Sexp {
    match fin.typ() {
        NILSXP | CLOSXP | BUILTINSXP | SPECIALSXP => {}
        _ => error_str("finalizer must be a function or NULL"),
    }
    new_weak_ref(key, val, fin, onexit != 0)
}

/// C finalizers live in a RAWSXP holding the function pointer.
unsafe fn c_finalizer(f: unsafe extern "C" fn(Sexp)) -> Sexp {
    let r = core().alloc_vector(RAWSXP, std::mem::size_of::<usize>());
    *(r.std_data() as *mut usize) = f as usize;
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_MakeWeakRefC(key: Sexp, val: Sexp, fin: unsafe extern "C" fn(Sexp), onexit: c_int) -> Sexp {
    Rf_protect(key);
    Rf_protect(val);
    let f = Rf_protect(c_finalizer(fin));
    let w = new_weak_ref(key, val, f, onexit != 0);
    Rf_unprotect(3);
    w
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_WeakRefKey(w: Sexp) -> Sexp {
    if w.typ() != WEAKREFSXP {
        error_str("not a weak reference")
    }
    weak_field(w, 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_WeakRefValue(w: Sexp) -> Sexp {
    if w.typ() != WEAKREFSXP {
        error_str("not a weak reference")
    }
    let v = weak_field(w, 1);
    if v != R_NilValue {
        ENSURE_NAMEDMAX(v);
    }
    v
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_RegisterFinalizerEx(s: Sexp, fun: Sexp, onexit: c_int) {
    R_MakeWeakRef(s, R_NilValue, fun, onexit);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_RegisterFinalizer(s: Sexp, fun: Sexp) {
    R_RegisterFinalizerEx(s, fun, 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_RegisterCFinalizerEx(s: Sexp, fun: unsafe extern "C" fn(Sexp), onexit: c_int) {
    R_MakeWeakRefC(s, R_NilValue, fun, onexit);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_RegisterCFinalizer(s: Sexp, fun: unsafe extern "C" fn(Sexp)) {
    R_RegisterCFinalizerEx(s, fun, 0)
}

/// Run a weak reference's finalizer once and clear it (memory.c
/// RunFinalizers / R_RunWeakRefFinalizer). Errors in R finalizers are
/// reported, not propagated.
pub unsafe fn run_finalizer(w: Sexp) {
    let key = weak_field(w, 0);
    let fin = weak_field(w, 2);
    set_weak_field(w, 0, R_NilValue);
    set_weak_field(w, 1, R_NilValue);
    set_weak_field(w, 2, R_NilValue);
    w.set_gp(w.gp() & !gp::READY_TO_FINALIZE);
    if fin == R_NilValue {
        return;
    }
    Rf_protect(key);
    Rf_protect(fin);
    if fin.typ() == RAWSXP {
        let f: unsafe extern "C" fn(Sexp) = std::mem::transmute(*(fin.std_data() as *const usize));
        crate::errors::protect_run(|| f(key));
    } else {
        let call = Rf_protect(crate::list::Rf_lang2(fin, key));
        let mut out = Sexp::NULL;
        let t = (crate::host::host().eval)(call, R_GlobalEnv, &mut out);
        if t != 0 {
            (crate::host::host().report)(t);
        }
        Rf_unprotect(1);
    }
    Rf_unprotect(2);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_RunWeakRefFinalizer(w: Sexp) {
    if w.typ() != WEAKREFSXP {
        error_str("not a weak reference")
    }
    run_finalizer(w)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_RunPendingFinalizers() {
    crate::util::run_finalizers()
}

/// Run the finalizers registered with `onexit = TRUE` (memory.c
/// R_RunExitFinalizers).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_RunExitFinalizers() {
    let refs: Vec<Sexp> = core().weak_refs().to_vec();
    for w in refs {
        if w.gp() & gp::FINALIZE_ON_EXIT != 0 {
            run_finalizer(w);
        }
    }
    crate::util::run_finalizers();
}
