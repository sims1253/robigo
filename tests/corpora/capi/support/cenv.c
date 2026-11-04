/* Bindings C makes in an environment it created (R_NewEnv + defineVar), as
 * rlang's env_clone() does for testthat's copy of a package namespace: they
 * are ordinary bindings, whatever the value. */
#include <R.h>
#include <Rinternals.h>

/* A copy of `from` with `from`'s parent: every binding is defined by C. */
SEXP capi_cenv_clone(SEXP from) {
    SEXP out = PROTECT(R_NewEnv(ENCLOS(from), TRUE, 29));
    SEXP nms = PROTECT(R_lsInternal3(from, TRUE, TRUE));
    for (int i = 0; i < LENGTH(nms); i++) {
        SEXP sym = installChar(STRING_ELT(nms, i));
        defineVar(sym, findVarInFrame3(from, sym, TRUE), out);
    }
    UNPROTECT(2);
    return out;
}

/* R_BindingIsActive for each name, as rlang's env_binding_types() asks. */
SEXP capi_cenv_active(SEXP env, SEXP nms) {
    SEXP out = PROTECT(allocVector(LGLSXP, LENGTH(nms)));
    for (int i = 0; i < LENGTH(nms); i++)
        LOGICAL(out)[i] = R_BindingIsActive(installChar(STRING_ELT(nms, i)), env);
    UNPROTECT(1);
    return out;
}
