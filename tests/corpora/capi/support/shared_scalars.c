// C writing into an argument must change only that object. robigo shares
// one object for each of TRUE, FALSE, NA and the small integers; a write
// into one of those (`i <- 0L` binds the shared 0L) must not reach every
// other `0L` in the session, as GNU R's per-literal objects never do.
#include <R.h>
#include <Rinternals.h>

SEXP capi_ss_set_int(SEXP x, SEXP v) {
    INTEGER(x)[0] = Rf_asInteger(v);
    return R_NilValue;
}

SEXP capi_ss_set_lgl(SEXP x) {
    LOGICAL(x)[0] = 0;
    return R_NilValue;
}

SEXP capi_ss_set_attr(SEXP x) {
    SEXP v = PROTECT(Rf_mkString("set in C"));
    Rf_setAttrib(x, Rf_install("tag"), v);
    UNPROTECT(1);
    return R_NilValue;
}
