// defineVar into an environment R_NewEnv created, with values C does not
// change in place (an environment, a symbol, a closure, a primitive): the
// bindings are plain bindings, as rlang's env_clone() makes them when it
// copies a namespace (testthat's test environment), so R_IsNamespaceEnv, topenv() and environmentName()
// see the copy as the namespace.
// R: main/envir.c:R_NewEnv, main/envir.c:defineVar, main/envir.c:R_IsNamespaceEnv
#include <R.h>
#include <Rinternals.h>

SEXP capi_newenv_bind(SEXP info, SEXP name, SEXP other, SEXP fun) {
    SEXP ns_sym = install(".__NAMESPACE__.");
    SEXP env = PROTECT(R_NewEnv(R_GlobalEnv, FALSE, 0));
    defineVar(ns_sym, info, env);
    defineVar(install(".packageName"), name, env);
    defineVar(install("an_env"), other, env);
    defineVar(install("a_sym"), install("abc"), env);
    defineVar(install("a_fun"), fun, env);
    defineVar(install("a_prim"), findFun(install("sum"), R_BaseEnv), env);
    SEXP ans = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 0, env);
    SET_VECTOR_ELT(ans, 1, ScalarLogical(R_IsNamespaceEnv(env)));
    SET_VECTOR_ELT(ans, 2, ScalarLogical(R_BindingIsActive(ns_sym, env)));
    UNPROTECT(2);
    return ans;
}
