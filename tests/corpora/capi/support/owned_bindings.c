// A vector C binds with defineVar into an environment R_NewEnv created
// and later changes in place (stats' numeric_deriv does this): R sees
// C's change, and the variable is a plain binding everywhere R can look
// (bindingIsActive, activeBindingFunction, makeActiveBinding,
// as.list.environment, eapply, serialize).
// R: main/envir.c:R_NewEnv, defineVar, FrameValues; main/serialize.c:WriteItem
#include <R.h>
#include <Rinternals.h>

static SEXP kept = NULL;

SEXP capi_owned_env(void) {
    SEXP env = PROTECT(R_NewEnv(R_GlobalEnv, FALSE, 0));
    SEXP v = PROTECT(allocVector(REALSXP, 2));
    REAL(v)[0] = 1;
    REAL(v)[1] = 2;
    defineVar(install("v"), v, env);
    if (kept) R_ReleaseObject(kept);
    kept = v;
    R_PreserveObject(kept);
    UNPROTECT(2);
    return env;
}

SEXP capi_owned_bump(void) {
    REAL(kept)[0] += 10;
    return R_NilValue;
}
