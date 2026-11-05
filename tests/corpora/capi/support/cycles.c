// Objects that only a runtime reference cycle, or only C, keeps alive.
// R: main/memory.c:RunGenCollect -- one collection finds everything
// unreachable, whichever objects refer to it.
#include <R.h>
#include <Rinternals.h>
#include <string.h>

static void cycles_finalizer(SEXP ptr) {
    Rprintf("finalizer of %s ran\n", (const char *) R_ExternalPtrAddr(ptr));
}

// An external pointer with a C finalizer (curl's handles).
SEXP cycles_make(SEXP name) {
    // the finalizer prints the name the address points to
    static char names[8][32];
    static int n = 0;
    char *slot = names[n++ % 8];
    strncpy(slot, CHAR(STRING_ELT(name, 0)), 31);
    SEXP ptr = PROTECT(R_MakeExternalPtr(slot, R_NilValue, R_NilValue));
    R_RegisterCFinalizerEx(ptr, cycles_finalizer, FALSE);
    UNPROTECT(1);
    return ptr;
}

// Keep `fun` only in the protected field of a new external pointer
// (curl's multi_add keeps its callbacks so).
SEXP cycles_hold(SEXP fun) {
    SEXP prot = PROTECT(allocVector(VECSXP, 1));
    SET_VECTOR_ELT(prot, 0, fun);
    SEXP ptr = PROTECT(R_MakeExternalPtr(NULL, R_NilValue, prot));
    UNPROTECT(2);
    return ptr;
}

// Call the function kept by cycles_hold with `arg`, as curl calls back.
SEXP cycles_call(SEXP ptr, SEXP arg) {
    SEXP fun = VECTOR_ELT(R_ExternalPtrProtected(ptr), 0);
    SEXP call = PROTECT(lang2(fun, arg));
    SEXP ans = eval(call, R_GlobalEnv);
    UNPROTECT(1);
    return ans;
}

// A pool that keeps a callback in its protected field, as curl's pools
// keep their handles' callbacks: a cycle through both heaps when the
// callback's frame holds the pool.
static void cycles_pool_finalizer(SEXP ptr) {
    Rprintf("finalizer of pool %s ran\n", (const char *) R_ExternalPtrAddr(ptr));
}

SEXP cycles_pool(SEXP name, SEXP fun) {
    static char names[8][32];
    static int n = 0;
    char *slot = names[n++ % 8];
    strncpy(slot, CHAR(STRING_ELT(name, 0)), 31);
    SEXP prot = PROTECT(allocVector(VECSXP, 1));
    SET_VECTOR_ELT(prot, 0, fun);
    SEXP ptr = PROTECT(R_MakeExternalPtr(slot, R_NilValue, prot));
    R_RegisterCFinalizerEx(ptr, cycles_pool_finalizer, FALSE);
    UNPROTECT(2);
    return ptr;
}

// A finalizer that allocates: the sweep's free slots are reused while
// the runtime still has the dead pool's wrapper.
static void cycles_busy_finalizer(SEXP ptr) {
    for (int i = 0; i < 2000; i++) {
        SEXP v = PROTECT(allocVector(INTSXP, 4));
        SEXP p = PROTECT(R_MakeExternalPtr(NULL, R_NilValue, v));
        UNPROTECT(2);
        (void) p;
    }
    Rprintf("busy finalizer ran\n");
}

SEXP cycles_busy_pool(SEXP fun) {
    SEXP prot = PROTECT(allocVector(VECSXP, 1));
    SET_VECTOR_ELT(prot, 0, fun);
    SEXP ptr = PROTECT(R_MakeExternalPtr(NULL, R_NilValue, prot));
    R_RegisterCFinalizerEx(ptr, cycles_busy_finalizer, FALSE);
    UNPROTECT(2);
    return ptr;
}
