// Atomic vectors reach C as the caller's own objects (GNU R passes them;
// robigo shares the buffer, docs/core.md §6): writes through INTEGER()
// and REAL() are what R reads next, during callbacks too. purrr's map
// writes its index into an argument before every call back into R, and
// dplyr writes the current group id into a vector it found in the mask.
#include <R.h>
#include <Rinternals.h>

// purrr's call_loop: write the counter, then evaluate a call that reads it
SEXP capi_zc_loop(SEXP call, SEXP env, SEXP counter) {
    int *p = INTEGER(counter);
    int n = 3;
    SEXP out = PROTECT(Rf_allocVector(VECSXP, n));
    for (int k = 0; k < n; ++k) {
        *p = k + 1;
        SET_VECTOR_ELT(out, k, R_forceAndCall(call, 1, env));
    }
    UNPROTECT(1);
    return out;
}

// dplyr's mask: the vector found by name, written before each eval
SEXP capi_zc_found(SEXP sym, SEXP call, SEXP env) {
    SEXP v = PROTECT(Rf_findVarInFrame(env, sym));
    double *p = REAL(v);
    SEXP out = PROTECT(Rf_allocVector(REALSXP, 2));
    p[0] = 10;
    REAL(out)[0] = Rf_asReal(Rf_eval(call, env));
    p[0] = 20;
    REAL(out)[1] = Rf_asReal(Rf_eval(call, env));
    UNPROTECT(2);
    return out;
}

// purrr's cleancall: the argument goes back to R inside a call C builds
SEXP capi_zc_reeval(SEXP fn, SEXP x, SEXP env) {
    SEXP call = PROTECT(Rf_lang2(fn, x));
    SEXP r = Rf_eval(call, env);
    UNPROTECT(1);
    return r;
}

// an element of a list argument
SEXP capi_zc_elt(SEXP lst) {
    INTEGER(VECTOR_ELT(lst, 0))[1] = 99;
    return R_NilValue;
}

// an attribute set on an argument
SEXP capi_zc_attr(SEXP x) {
    Rf_setAttrib(x, Rf_install("tag"), Rf_mkString("set in C"));
    return R_NilValue;
}

// a logical argument turned into integer codes in place, as utils'
// typeconvert does with duplicated()'s result (io.c: SET_TYPEOF(dup, INTSXP))
SEXP capi_zc_retype(SEXP x) {
    SET_TYPEOF(x, INTSXP);
    INTEGER(x)[0] = 7;
    return x;
}
