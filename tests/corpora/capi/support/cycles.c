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
