/* A routine that writes into its argument and returns it (Rcpp's
 * `IntegerVector x(vec); x[i] *= 2; return x;`). GNU R hands C the
 * caller's own object: a compact sequence expands on a writable data
 * pointer and keeps C's writes and attributes; a character vector C
 * preserves (Rcpp_precious_preserve's list) and writes through the
 * pointer DATAPTR_RO gave (Rcpp's dataptr callable, which x.sort()
 * uses) is changed for the caller too. */
#include <R.h>
#include <Rinternals.h>

static SEXP precious = NULL;

/* Rcpp_precious_preserve / Rcpp_precious_remove: a doubly-linked list
 * of cells in one preserved pairlist, the object in the cell's TAG */
static SEXP keep(SEXP x) {
    if (precious == NULL) {
        precious = Rf_cons(R_NilValue, R_NilValue);
        R_PreserveObject(precious);
    }
    PROTECT(x);
    SEXP cell = PROTECT(Rf_cons(precious, CDR(precious)));
    SET_TAG(cell, x);
    SETCDR(precious, cell);
    if (CDR(cell) != R_NilValue) SETCAR(CDR(cell), cell);
    UNPROTECT(2);
    return cell;
}

static void release(SEXP cell) {
    SEXP before = CAR(cell), after = CDR(cell);
    SETCDR(before, after);
    if (after != R_NilValue) SETCAR(after, before);
}

SEXP capi_twice_int(SEXP x) {
    int *p = INTEGER(x);
    for (R_xlen_t i = 0; i < XLENGTH(x); i++) p[i] *= 2;
    return x;
}

SEXP capi_twice_real(SEXP x) {
    double *p = REAL(x);
    for (R_xlen_t i = 0; i < XLENGTH(x); i++) p[i] *= 2;
    return x;
}

SEXP capi_fill_ro(SEXP x, SEXP v) {
    if (XLENGTH(v) < 1)
        error("capi_fill_ro: need a non-empty fill value");
    SEXP cell = keep(x);
    int *p = (int *) DATAPTR_RO(x);
    for (R_xlen_t i = 0; i < XLENGTH(x); i++) p[i] = INTEGER(v)[0];
    release(cell);
    return x;
}

SEXP capi_set_names(SEXP x, SEXP nm) {
    SEXP cell = keep(x);
    Rf_setAttrib(x, R_NamesSymbol, nm);
    release(cell);
    return x;
}

/* reverse a character vector in place through DATAPTR_RO's pointer */
SEXP capi_rev_str_ro(SEXP x) {
    SEXP cell = keep(x);
    SEXP *p = (SEXP *) DATAPTR_RO(x);
    R_xlen_t n = XLENGTH(x);
    for (R_xlen_t i = 0; i < n / 2; i++) {
        SEXP t = p[i];
        p[i] = p[n - 1 - i];
        p[n - 1 - i] = t;
    }
    release(cell);
    return x;
}

SEXP capi_negzero_real(SEXP x) {
    REAL(x)[0] = -0.0;
    return R_NilValue;
}
