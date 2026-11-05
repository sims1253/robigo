/* rlang's zap: C keeps a looked-up object in a static and compares
   objects that come back from R with it by address. GNU R's objects keep
   their identity through C and back, also inside a list C builds (rlang's
   list3()). */
#include <R.h>
#include <Rinternals.h>

static SEXP zap = NULL;

SEXP capi_rt_init(SEXP env)
{
    zap = findVarInFrame(env, install("zap"));
    return R_NilValue;
}

SEXP capi_rt_get(void) { return zap; }

/* a list built in C around an object from R */
SEXP capi_rt_wrap(SEXP x)
{
    SEXP out = PROTECT(allocVector(VECSXP, 1));
    SET_VECTOR_ELT(out, 0, x);
    UNPROTECT(1);
    return out;
}

SEXP capi_rt_is(SEXP x) { return ScalarLogical(x == zap); }

SEXP capi_rt_first_is(SEXP x) { return ScalarLogical(VECTOR_ELT(x, 0) == zap); }
