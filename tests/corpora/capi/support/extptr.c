// External pointers, finalizers and weak references.
// R: main/memory.c:R_MakeExternalPtr, main/weakref.c
#include <R.h>
#include <Rinternals.h>

static int finalizers_run = 0;

// R_CFinalizer_t: called by the gc (or R_RunPendingFinalizers) with the
// (cleared) external pointer.
static void capi_finalizer(SEXP ptr) {
    finalizers_run++;
    Rprintf("finalizer %d ran (addr is %s)\n",
            finalizers_run, R_ExternalPtrAddr(ptr) == NULL ? "null" : "kept");
}

// Make an external pointer with a tag and a protected value; the setters
// and getters move and clear parts of it.
SEXP capi_extptr(void) {
    SEXP ptr = PROTECT(R_MakeExternalPtr((void *)0x1234, install("tag1"),
                                         ScalarInteger(7)));
    Rboolean has_addr = R_ExternalPtrAddr(ptr) != NULL;
    SEXP tag1 = PROTECT(ScalarString(PRINTNAME(R_ExternalPtrTag(ptr))));
    SEXP prot1 = PROTECT(ScalarInteger(asInteger(R_ExternalPtrProtected(ptr))));
    R_SetExternalPtrTag(ptr, install("tag2"));
    R_SetExternalPtrProtected(ptr, ScalarInteger(8));
    SEXP tag2 = PROTECT(ScalarString(PRINTNAME(R_ExternalPtrTag(ptr))));
    R_ClearExternalPtr(ptr);
    SEXP fresh = PROTECT(R_MakeExternalPtr(NULL, install("tag3"), R_NilValue));
    R_SetExternalPtrAddr(fresh, (void *)0x777);
    SEXP ans = PROTECT(allocVector(VECSXP, 6));
    SET_VECTOR_ELT(ans, 0, ScalarLogical(has_addr));
    SET_VECTOR_ELT(ans, 1, tag1);
    SET_VECTOR_ELT(ans, 2, prot1);
    SET_VECTOR_ELT(ans, 3, tag2);
    SET_VECTOR_ELT(ans, 4, ScalarLogical(R_ExternalPtrAddr(ptr) == NULL
                                         && R_ExternalPtrAddrFn(ptr) == NULL));
    SET_VECTOR_ELT(ans, 5, ScalarLogical(R_ExternalPtrAddr(fresh) == (void *)0x777));
    UNPROTECT(7);
    return ans;
}

// A pointer with a finalizer; the R side drops it and calls gc().
SEXP capi_make_finalized(void) {
    SEXP ptr = PROTECT(R_MakeExternalPtr((void *)0x5678, R_NilValue, R_NilValue));
    R_RegisterCFinalizerEx(ptr, capi_finalizer, TRUE);
    UNPROTECT(1);
    return ptr;
}

// R_MakeWeakRef / R_WeakRefKey / R_WeakRefValue; an R-level finalizer
// function can be attached (run by gc like the C one).
SEXP capi_weakref(SEXP key, SEXP val) {
    SEXP wr = PROTECT(R_MakeWeakRef(key, val, R_NilValue, FALSE));
    SEXP ans = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 0, ScalarLogical(R_WeakRefKey(wr) == key));
    SET_VECTOR_ELT(ans, 1, R_WeakRefValue(wr));
    SET_VECTOR_ELT(ans, 2, ScalarInteger(finalizers_run));
    UNPROTECT(2);
    return ans;
}
