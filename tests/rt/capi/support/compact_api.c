// base's compact sequences (altclasses.c) handed to the C API's setters
// and copiers: setAttrib's dim, tsp and row.names read the value through
// INTEGER or REAL, which expands it (the class's Dataptr); lengthgets
// copies it by element.
#include <R.h>
#include <Rinternals.h>

// whether DATAPTR_OR_NULL has the data (an ALTREP sequence: expanded)
SEXP rt_ca_expanded(SEXP x) {
    return ScalarLogical(DATAPTR_OR_NULL(x) != NULL);
}

// a fresh integer vector of n elements with the dim d
SEXP rt_ca_dim(SEXP n, SEXP d) {
    SEXP v = PROTECT(allocVector(INTSXP, asInteger(n)));
    for (R_xlen_t i = 0; i < XLENGTH(v); i++) INTEGER(v)[i] = (int) i;
    setAttrib(v, R_DimSymbol, d);
    UNPROTECT(1);
    return v;
}

// a fresh double vector of n elements with the tsp t
SEXP rt_ca_tsp(SEXP n, SEXP t) {
    SEXP v = PROTECT(allocVector(REALSXP, asInteger(n)));
    for (R_xlen_t i = 0; i < XLENGTH(v); i++) REAL(v)[i] = (double) i;
    setAttrib(v, R_TspSymbol, t);
    UNPROTECT(1);
    return v;
}

// a list of one element with the row names rn
SEXP rt_ca_rownames(SEXP rn) {
    SEXP l = PROTECT(allocVector(VECSXP, 1));
    SET_VECTOR_ELT(l, 0, allocVector(INTSXP, 0));
    setAttrib(l, R_RowNamesSymbol, rn);
    UNPROTECT(1);
    return l;
}

SEXP rt_ca_lengthgets(SEXP x, SEXP n) {
    return lengthgets(x, asInteger(n));
}
