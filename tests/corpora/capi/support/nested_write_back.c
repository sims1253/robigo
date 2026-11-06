// Native calls inside a native call (C code runs R code with Rf_eval, and
// that R code calls routines): what each one writes is the caller's as it
// returns, and its cost is that of what it is handed.
// R: main/dotcode.c (do_dotcall), main/eval.c
#include <R.h>
#include <Rinternals.h>

// Hold `x` while R code `fun()` runs (rlang's C eval holds a whole test).
SEXP capi_nest_outer(SEXP x, SEXP fun) {
    SEXP call = PROTECT(lang1(fun));
    Rf_eval(call, R_GlobalEnv);
    UNPROTECT(1);
    return ScalarInteger(LENGTH(x));
}

SEXP capi_nest_inner(SEXP x) {
    return ScalarInteger(LENGTH(x));
}

// Write into the first element of a list.
SEXP capi_nest_poke(SEXP x) {
    INTEGER(VECTOR_ELT(x, 0))[0] = 99;
    return R_NilValue;
}
