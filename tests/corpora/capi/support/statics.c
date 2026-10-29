/* Objects C code keeps in statics across calls (Rcpp's .rcpp_cache and
   Rcpp::Function): a vector it stored in an environment and read back
   with findVarInFrame, and functions it looked up with findFun. GNU R
   keeps them alive through their bindings, so they must survive gc(). */
#include <R.h>
#include <Rinternals.h>

static SEXP cache = NULL, base_fun = NULL, user_fun = NULL;

SEXP capi_statics_init(SEXP env)
{
    SEXP v = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(v, 0, env);
    SET_VECTOR_ELT(v, 1, ScalarInteger(7));
    defineVar(install(".cache"), v, env);
    UNPROTECT(1);
    cache = findVarInFrame(env, install(".cache"));
    base_fun = findFun(install("paste"), R_BaseEnv);
    user_fun = findFun(install("twice"), R_GlobalEnv);
    return R_NilValue;
}

SEXP capi_statics_use(void)
{
    /* the C side's own writes to the stored vector persist */
    SET_VECTOR_ELT(cache, 1, ScalarInteger(INTEGER(VECTOR_ELT(cache, 1))[0] + 1));
    SEXP n = PROTECT(eval(PROTECT(lang2(user_fun, VECTOR_ELT(cache, 1))), VECTOR_ELT(cache, 0)));
    SEXP r = eval(PROTECT(lang3(base_fun, mkString("n ="), n)), VECTOR_ELT(cache, 0));
    UNPROTECT(3);
    return r;
}
