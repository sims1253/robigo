// A variable's value is an object C can write into, and R reads the
// write through the variable (GNU R hands C the binding's own value, the
// VM's unboxed binding cells included: envir.c BINDING_VALUE ->
// memory.c R_expand_binding_value). purrr's map writes its loop index
// `i` in place before each callback that evaluates `.f(.x[[i]], ...)`.
#include <R.h>
#include <Rinternals.h>

static void put(SEXP x, SEXP v, int k) {
    if (TYPEOF(x) == INTSXP)
        INTEGER(x)[0] = Rf_asInteger(v) + k;
    else if (TYPEOF(x) == REALSXP)
        REAL(x)[0] = Rf_asReal(v) + k;
    else if (TYPEOF(x) == LGLSXP)
        LOGICAL(x)[0] = Rf_asLogical(v);
    else
        Rf_error("not a number");
}

// write `v` into the argument, no callback
SEXP capi_id_set(SEXP x, SEXP v) {
    put(x, v, 0);
    return R_NilValue;
}

// write `start + k` into the counter, then evaluate `call` in `env`
SEXP capi_id_loop(SEXP env, SEXP call, SEXP counter, SEXP start, SEXP n) {
    int len = Rf_asInteger(n);
    SEXP out = PROTECT(Rf_allocVector(VECSXP, len));
    for (int k = 0; k < len; ++k) {
        put(counter, start, k);
        SET_VECTOR_ELT(out, k, Rf_eval(call, env));
    }
    UNPROTECT(1);
    return out;
}

// purrr's cleancall + map_impl: the arguments come in a pairlist built
// from `...`; the index goes 1..n, each call forced as R_forceAndCall
SEXP capi_id_dots(SEXP args) {
    SEXP env = CAR(args), call = CADR(args), i = CADDR(args);
    int n = Rf_asInteger(CADDDR(args));
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
    SEXP x = Rf_findVar(sym, env);
    if (TYPEOF(x) == PROMSXP)
        x = Rf_eval(x, env);
    put(x, v, 0);
    return R_NilValue;
}
