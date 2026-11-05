/* Writes C makes without a setter: through a pointer it took once
   (rlang's sys_frame_call: `*n_addr = n` into the argument of a call it
   keeps, then eval), and SETLENGTH on an argument (rlang's dynamic arrays
   resize in place). GNU R's objects are C's, so R sees both. */
#include <R.h>
#include <Rinternals.h>

static SEXP call = NULL;
static int *n_addr = NULL;

SEXP capi_raw_init(SEXP env)
{
    SEXP n = PROTECT(ScalarInteger(0));
    n_addr = INTEGER(n);
    call = PROTECT(lang2(install("identity"), n));
    R_PreserveObject(call);
    /* R holds the call too */
    defineVar(install("cached_call"), call, env);
    UNPROTECT(2);
    return R_NilValue;
}

SEXP capi_raw_eval(SEXP n)
{
    *n_addr = INTEGER(n)[0];
    return eval(call, R_BaseEnv);
}

SEXP capi_raw_shrink(SEXP x, SEXP n)
{
    SETLENGTH(x, INTEGER(n)[0]);
    return R_NilValue;
}
