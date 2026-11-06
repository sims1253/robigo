// A variable's value is an object C can write into, and R reads the
// write through the variable (GNU R hands C the binding's own value, the
// VM's unboxed binding cells included: envir.c BINDING_VALUE ->
// memory.c R_expand_binding_value). purrr's map writes its loop index
// `i` in place before each callback that evaluates `.f(.x[[i]], ...)`.
#include <R.h>
#include <Rinternals.h>

// write `v + k` into the scalar `x` (a logical takes `v` as it is: it is
// not a counter); NA stays NA
static void put(const char *who, SEXP x, SEXP v, int k) {
    if (Rf_length(x) != 1)
        Rf_error("%s: not a scalar", who);
    if (TYPEOF(x) == INTSXP) {
        int a = Rf_asInteger(v);
        INTEGER(x)[0] = a == NA_INTEGER ? NA_INTEGER : a + k;
    } else if (TYPEOF(x) == REALSXP)
        REAL(x)[0] = Rf_asReal(v) + k;
    else if (TYPEOF(x) == LGLSXP)
        LOGICAL(x)[0] = Rf_asLogical(v);
    else
        Rf_error("%s: not a number", who);
}

static int count(const char *who, SEXP n) {
    int len = Rf_asInteger(n);
    if (len == NA_INTEGER || len < 0)
        Rf_error("%s: invalid count", who);
    return len;
}

static void need(const char *who, int ok, const char *what) {
    if (!ok)
        Rf_error("%s: %s", who, what);
}

// write `v` into the argument, no callback
SEXP capi_id_set(SEXP x, SEXP v) {
    put("capi_id_set", x, v, 0);
    return R_NilValue;
}

// write `start + k` into the counter, then evaluate `call` in `env`
SEXP capi_id_loop(SEXP env, SEXP call, SEXP counter, SEXP start, SEXP n) {
    need("capi_id_loop", TYPEOF(env) == ENVSXP, "not an environment");
    int len = count("capi_id_loop", n);
    SEXP out = PROTECT(Rf_allocVector(VECSXP, len));
    for (int k = 0; k < len; ++k) {
        put("capi_id_loop", counter, start, k);
        SET_VECTOR_ELT(out, k, Rf_eval(call, env));
    }
    UNPROTECT(1);
    return out;
}

// purrr's cleancall + map_impl: the arguments come in a pairlist built
// from `...`; the index goes 1..n, each call forced as R_forceAndCall
SEXP capi_id_dots(SEXP args) {
    need("capi_id_dots", Rf_length(args) == 4, "needs (env, call, i, n)");
    SEXP env = CAR(args), call = CADR(args), i = CADDR(args);
    need("capi_id_dots", TYPEOF(env) == ENVSXP, "not an environment");
    need("capi_id_dots", TYPEOF(call) == LANGSXP, "not a call");
    need("capi_id_dots", TYPEOF(i) == INTSXP && Rf_length(i) == 1, "not an integer scalar");
    int n = count("capi_id_dots", CADDDR(args));
    int *p = INTEGER(i);
    SEXP out = PROTECT(Rf_allocVector(VECSXP, n));
    for (int k = 0; k < n; ++k) {
        *p = k + 1;
        SET_VECTOR_ELT(out, k, R_forceAndCall(call, 1, env));
    }
    UNPROTECT(1);
    return out;
}

// the variable found by name, not passed
SEXP capi_id_findvar(SEXP sym, SEXP env, SEXP v) {
    need("capi_id_findvar", TYPEOF(sym) == SYMSXP, "not a symbol");
    need("capi_id_findvar", TYPEOF(env) == ENVSXP, "not an environment");
    SEXP x = Rf_findVar(sym, env);
    if (x == R_UnboundValue || x == R_MissingArg)
        Rf_error("capi_id_findvar: object '%s' not found", CHAR(PRINTNAME(sym)));
    if (TYPEOF(x) == PROMSXP)
        x = Rf_eval(x, env);
    put("capi_id_findvar", x, v, 0);
    return R_NilValue;
}

// R_forceAndCall: the closure's first `n` arguments are forced before the
// call, as purrr's map forces `.f(.x[[i]], ...)` while `i` is the index
SEXP capi_id_force(SEXP call, SEXP env, SEXP n) {
    need("capi_id_force", TYPEOF(call) == LANGSXP, "not a call");
    need("capi_id_force", TYPEOF(env) == ENVSXP, "not an environment");
    return R_forceAndCall(call, count("capi_id_force", n), env);
}
