// A package ALTREP class whose Length method allocates and re-enters R
// (docs/g3.md §15.3 E6): data1 is an R function of no arguments, and the
// length is what it returns.
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Altrep.h>

static R_altrep_class_t len_class;
static int length_calls = 0;

static R_xlen_t len_method(SEXP x) {
    length_calls++;
    // a large allocation (a collection may run here) and a call into R
    SEXP junk = PROTECT(allocVector(REALSXP, 100000));
    REAL(junk)[0] = 1;
    SEXP call = PROTECT(lang1(R_altrep_data1(x)));
    SEXP v = PROTECT(eval(call, R_GlobalEnv));
    R_xlen_t n = (R_xlen_t) asInteger(v);
    UNPROTECT(3);
    return n;
}

SEXP rt_altrep_init(void) {
    len_class = R_make_altinteger_class("rt_len", "rtcapi", NULL);
    R_set_altrep_Length_method(len_class, len_method);
    return R_NilValue;
}

SEXP rt_altrep_new(SEXP fun) {
    return R_new_altrep(len_class, fun, R_NilValue);
}

SEXP rt_altrep_calls(void) {
    return ScalarInteger(length_calls);
}

// R_altrep_inherits, R_altrep_data1 and ALTREP of an object
SEXP rt_altrep_info(SEXP x) {
    SEXP ans = PROTECT(allocVector(LGLSXP, 3));
    LOGICAL(ans)[0] = ALTREP(x);
    LOGICAL(ans)[1] = ALTREP(x) && R_altrep_inherits(x, len_class);
    LOGICAL(ans)[2] = ALTREP(x) && TYPEOF(R_altrep_data1(x)) == CLOSXP;
    UNPROTECT(1);
    return ans;
}

// a class with no Length method: GNU R's default is an error
SEXP rt_altrep_no_length(void) {
    R_altrep_class_t c = R_make_altinteger_class("rt_nolen", "rtcapi", NULL);
    return R_new_altrep(c, R_NilValue, R_NilValue);
}
