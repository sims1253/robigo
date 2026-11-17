// base's compact sequences (altclasses.c) seen from package C: the
// length, the elements, DATAPTR_OR_NULL before and after DATAPTR expands
// one, and a write through INTEGER into the expansion.
#include <R.h>
#include <Rinternals.h>

// ALTREP, LENGTH, XLENGTH, and whether DATAPTR_OR_NULL has the data
SEXP rt_cs_info(SEXP x) {
    SEXP ans = PROTECT(allocVector(INTSXP, 4));
    INTEGER(ans)[0] = ALTREP(x);
    INTEGER(ans)[1] = LENGTH(x);
    INTEGER(ans)[2] = (int) XLENGTH(x);
    INTEGER(ans)[3] = DATAPTR_OR_NULL(x) != NULL;
    UNPROTECT(1);
    return ans;
}

// the elements i (0-based) through INTEGER_ELT or REAL_ELT, as doubles
SEXP rt_cs_elts(SEXP x, SEXP idx) {
    R_xlen_t n = XLENGTH(idx);
    SEXP ans = PROTECT(allocVector(REALSXP, n));
    for (R_xlen_t k = 0; k < n; k++) {
        R_xlen_t i = (R_xlen_t) INTEGER(idx)[k];
        REAL(ans)[k] = TYPEOF(x) == INTSXP ? (double) INTEGER_ELT(x, i) : REAL_ELT(x, i);
    }
    UNPROTECT(1);
    return ans;
}

// INTEGER_ELT(x, i) whatever x is
SEXP rt_cs_int_elt(SEXP x, SEXP i) {
    return ScalarInteger(INTEGER_ELT(x, (R_xlen_t) asInteger(i)));
}

// the sum of the elements read through INTEGER or REAL (DATAPTR expands)
SEXP rt_cs_dataptr_sum(SEXP x) {
    double s = 0;
    R_xlen_t n = XLENGTH(x);
    if (TYPEOF(x) == INTSXP) {
        int *p = INTEGER(x);
        for (R_xlen_t i = 0; i < n; i++) s += p[i];
    } else {
        double *p = REAL(x);
        for (R_xlen_t i = 0; i < n; i++) s += p[i];
    }
    return ScalarReal(s);
}

// the sum read through DATAPTR_RO
SEXP rt_cs_ro_sum(SEXP x) {
    double s = 0;
    R_xlen_t n = XLENGTH(x);
    const void *p = DATAPTR_RO(x);
    for (R_xlen_t i = 0; i < n; i++)
        s += TYPEOF(x) == INTSXP ? ((const int *) p)[i] : ((const double *) p)[i];
    return ScalarReal(s);
}

// INTEGER(x)[i] = v: C writing into the vector it was handed
SEXP rt_cs_write(SEXP x, SEXP i, SEXP v) {
    INTEGER(x)[asInteger(i)] = asInteger(v);
    return R_NilValue;
}

// DATAPTR_RO of an object that is not a vector
SEXP rt_cs_dataptr_of(SEXP x) {
    (void) DATAPTR_RO(x);
    return R_NilValue;
}
