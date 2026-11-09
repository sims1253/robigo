/* C writes an attribute's cells through the pointer DATAPTR_RO gave,
 * in a later call than the one that bound the object. No setter runs, so
 * nothing reports the write, and the names are neither the call's
 * argument nor an atomic vector. GNU R's attribute is the object C wrote
 * into, so R sees the swap. */
#include <R.h>
#include <Rinternals.h>

/* expands x (INTEGER()), names it and binds it as `bound`: the object
 * the later calls are handed */
SEXP capi_bind_named(SEXP x, SEXP env) {
    if (TYPEOF(x) != INTSXP || XLENGTH(x) < 2)
        error("capi_bind_named: need an integer vector of length two or more");
    INTEGER(x)[0] += 100;
    R_xlen_t n = XLENGTH(x);
    SEXP nm = PROTECT(Rf_allocVector(STRSXP, n));
    for (R_xlen_t i = 0; i < n; i++) {
        char b[2] = {(char) ('a' + i % 26), 0};
        SET_STRING_ELT(nm, i, mkChar(b));
    }
    Rf_setAttrib(x, R_NamesSymbol, nm);
    Rf_defineVar(Rf_install("bound"), x, env);
    UNPROTECT(1);
    return R_NilValue;
}

/* swaps the first two names through DATAPTR_RO's pointer (Rcpp's
 * CharacterVector over names(x), then sort()) */
SEXP capi_swap_names_ro(SEXP x) {
    SEXP nm = Rf_getAttrib(x, R_NamesSymbol);
    if (TYPEOF(nm) != STRSXP || XLENGTH(nm) < 2)
        error("capi_swap_names_ro: need two or more names");
    /* deliberate: a write through DATAPTR_RO's const pointer */
    SEXP *p = (SEXP *) DATAPTR_RO(nm);
    SEXP t = p[0];
    p[0] = p[1];
    p[1] = t;
    return R_NilValue;
}
