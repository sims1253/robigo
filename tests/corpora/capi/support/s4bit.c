// The S4 bit of objects that reach C by reference (closures,
// environments) or as cells (calls, pairlists): IS_S4_OBJECT reads it,
// and a routine that only looks must leave it set (rlang's env_clone
// copies an S4 generic function this way; methods then reads its slots).
// R: Rinternals.h:IS_S4_OBJECT, envir.c:R_getVarEx
#include <R.h>
#include <Rinternals.h>

SEXP capi_is_s4(SEXP x) {
    return ScalarLogical(IS_S4_OBJECT(x) != 0);
}

SEXP capi_identity(SEXP x) {
    return x;
}

SEXP capi_get_var(SEXP sym, SEXP env) {
    return R_getVarEx(sym, env, FALSE, R_NilValue);
}
