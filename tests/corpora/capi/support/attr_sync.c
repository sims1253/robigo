// The attributes of environments and external pointers belong to the object:
// C and R see each other's writes across calls, whichever made the object
// and whichever saw it first.
// R: main/attrib.c
#include <R.h>
#include <Rinternals.h>

// Give C its handle on `x` before R classes it (rlang's env_bind0).
SEXP capi_attr_touch(SEXP x) {
    return R_NilValue;
}

// A routine that never sees `x`.
SEXP capi_attr_unrelated(void) {
    return ScalarInteger(1);
}

SEXP capi_attr_read(SEXP x, SEXP name) {
    return Rf_getAttrib(x, install(CHAR(STRING_ELT(name, 0))));
}

SEXP capi_attr_object(SEXP x) {
    return ScalarLogical(OBJECT(x));
}

SEXP capi_attr_set(SEXP x, SEXP name, SEXP value) {
    Rf_setAttrib(x, install(CHAR(STRING_ELT(name, 0))), value);
    return R_NilValue;
}

SEXP capi_attr_extptr(void) {
    return R_MakeExternalPtr(NULL, R_NilValue, R_NilValue);
}

// R code changes `x` while the routine runs and the routine leaves it alone.
SEXP capi_attr_callback(SEXP fun, SEXP x) {
    SEXP call = PROTECT(lang2(fun, x));
    Rf_eval(call, R_GlobalEnv);
    UNPROTECT(1);
    return R_NilValue;
}
