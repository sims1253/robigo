// Promises assembled in C (rlang's env_clone of a namespace rebuilds its
// lazy bindings this way: env-binding.c r_env_bind_delayed's
// Rf_allocSExp(PROMSXP) + SET_PRCODE/SET_PRENV + Rf_defineVar).
#include <R.h>
#include <Rinternals.h>

// The unforced path: a promise whose forcing evaluates PRCODE in PRENV.
SEXP capi_promise_raw(SEXP expr, SEXP env) {
    SEXP p = PROTECT(Rf_allocSExp(PROMSXP));
    SET_PRCODE(p, expr);
    SET_PRENV(p, env);
    SET_PRVALUE(p, R_UnboundValue);
    SEXP out = PROTECT(R_NewEnv(env, 1, 29));
    defineVar(install("v"), p, out);
    UNPROTECT(2);
    return out;
}

// The forced path: PRVALUE set, PRENV NULL, as a forced promise looks.
SEXP capi_promise_forced(SEXP value) {
    SEXP expr = PROTECT(Rf_lang1(install("identity")));
    SEXP p = PROTECT(Rf_allocSExp(PROMSXP));
    SET_PRCODE(p, expr);
    SET_PRENV(p, R_NilValue);
    SET_PRVALUE(p, value);
    SEXP out = PROTECT(R_NewEnv(R_BaseEnv, 1, 29));
    defineVar(install("v"), p, out);
    UNPROTECT(3);
    return out;
}

// R_getVar forces a promise binding and hands the value to C.
SEXP capi_promise_getvar_typeof(SEXP env) {
    SEXP sym = PROTECT(install("v"));
    SEXP v = PROTECT(R_getVar(sym, env, FALSE));
    int t = TYPEOF(v);
    UNPROTECT(2);
    return ScalarInteger(t);
}
