#include <R.h>
#include <Rinternals.h>

/* An environment made by R_NewEnv, filled with defineVar: a value of each
   kind, as rlang's env_clone() and new_environment() do. */
SEXP capi_c_env_fill(SEXP parent, SEXP values) {
    SEXP env = PROTECT(R_NewEnv(parent, TRUE, 29));
    if (TYPEOF(values) != VECSXP)
        error("capi_c_env_fill: 'values' must be a list");
    SEXP names = getAttrib(values, R_NamesSymbol);
    if (TYPEOF(names) != STRSXP || XLENGTH(names) < XLENGTH(values))
        error("capi_c_env_fill: 'values' needs a name for each element");
    for (R_xlen_t i = 0; i < XLENGTH(values); i++)
        defineVar(installChar(STRING_ELT(names, i)), VECTOR_ELT(values, i), env);
    UNPROTECT(1);
    return env;
}

/* R_BindingIsActive of each name, as C sees it. */
SEXP capi_c_env_active(SEXP env, SEXP names) {
    if (TYPEOF(names) != STRSXP)
        error("capi_c_env_active: 'names' must be a character vector");
    R_xlen_t n = XLENGTH(names);
    SEXP out = PROTECT(allocVector(LGLSXP, n));
    for (R_xlen_t i = 0; i < n; i++)
        LOGICAL(out)[i] = R_BindingIsActive(installChar(STRING_ELT(names, i)), env);
    UNPROTECT(1);
    return out;
}

/* A double C keeps writing into after binding it (stats' numericDeriv). */
SEXP capi_c_env_poke(SEXP env, SEXP sym, SEXP value) {
    SEXP cur = findVarInFrame(env, sym);
    if (TYPEOF(cur) != REALSXP || XLENGTH(cur) < 1)
        error("capi_c_env_poke: the binding is not a non-empty double");
    double *p = REAL(cur);
    p[0] = asReal(value);
    return R_NilValue;
}

/* A copy of a namespace made in C, as rlang::env_clone() and testthat's
   test environment (env_clone(asNamespace(pkg))) are. */
SEXP capi_c_env_clone(SEXP from) {
    SEXP env = PROTECT(R_NewEnv(ENCLOS(from), TRUE, 29));
    SEXP names = PROTECT(R_lsInternal3(from, TRUE, FALSE));
    for (R_xlen_t i = 0; i < XLENGTH(names); i++) {
        SEXP sym = installChar(STRING_ELT(names, i));
        /* the forced value stays protected across defineVar, which
           allocates the binding cell */
        SEXP v = PROTECT(findVarInFrame(from, sym));
        if (TYPEOF(v) == PROMSXP)
            v = eval(v, from);
        PROTECT(v);
        defineVar(sym, v, env);
        UNPROTECT(2);
    }
    UNPROTECT(2);
    return env;
}
