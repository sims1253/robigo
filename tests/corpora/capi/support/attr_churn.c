// A vector C binds in an environment and later receives back: its
// attributes are the object's, read and written by C in place, and a call
// that leaves them alone leaves them alone.
// R: main/attrib.c, main/envir.c
#include <R.h>
#include <Rinternals.h>

// Bind `name` in `env` to a double vector of length n with names "a1".."an"
// (rlang's and S7's C code bind what they build this way).
SEXP capi_churn_store(SEXP env, SEXP name, SEXP n) {
    int len = asInteger(n);
    SEXP x = PROTECT(allocVector(REALSXP, len));
    SEXP nm = PROTECT(allocVector(STRSXP, len));
    char buf[32];
    for (int i = 0; i < len; i++) {
        REAL(x)[i] = i + 1;
        snprintf(buf, sizeof buf, "a%d", i + 1);
        SET_STRING_ELT(nm, i, mkChar(buf));
    }
    setAttrib(x, R_NamesSymbol, nm);
    defineVar(installChar(STRING_ELT(name, 0)), x, env);
    UNPROTECT(2);
    return R_NilValue;
}

// A routine that only reads its argument.
SEXP capi_churn_touch(SEXP x) {
    return ScalarInteger(LENGTH(x));
}

SEXP capi_churn_setattr(SEXP x, SEXP name, SEXP value) {
    setAttrib(x, installChar(STRING_ELT(name, 0)), value);
    return R_NilValue;
}

// Write into the names vector itself.
SEXP capi_churn_poke_name(SEXP x, SEXP i, SEXP s) {
    SET_STRING_ELT(getAttrib(x, R_NamesSymbol), asInteger(i) - 1, STRING_ELT(s, 0));
    return R_NilValue;
}
