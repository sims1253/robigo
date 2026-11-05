/* Values C code looked up keeps in statics without protecting them, as
   rlang does with `rlang_zap = rlang_ns_get("zap!")` and compares
   arguments against later (`value == rlang_zap` in ffi_env_bind). GNU R
   keeps such an object alive through its binding, and an argument that
   is the bound value is the same object. */
#include <R.h>
#include <Rinternals.h>

static SEXP by_frame = NULL, by_getvar = NULL;

SEXP capi_lookup_init(SEXP env)
{
    by_frame = findVarInFrame(env, install("marker"));
    by_getvar = R_getVar(install("pair"), env, FALSE);
    return R_NilValue;
}

/* allocate and drop small objects, so freed slots of these sizes are
   reused */
SEXP capi_lookup_churn(SEXP n)
{
    for (int i = 0; i < INTEGER(n)[0]; i++) {
        SEXP x = PROTECT(allocVector(VECSXP, i % 3));
        if (i % 3)
            SET_VECTOR_ELT(x, 0, ScalarInteger(i));
        SEXP y = PROTECT(allocVector(INTSXP, 2 + i % 3));
        for (int k = 0; k < LENGTH(y); k++)
            INTEGER(y)[k] = 2 * k;
        UNPROTECT(2);
    }
    return R_NilValue;
}

SEXP capi_lookup_cached(void)
{
    SEXP out = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(out, 0, by_frame);
    SET_VECTOR_ELT(out, 1, by_getvar);
    UNPROTECT(1);
    return out;
}

SEXP capi_lookup_is(SEXP x, SEXP y)
{
    return ScalarLogical(x == by_frame && y == by_getvar);
}
