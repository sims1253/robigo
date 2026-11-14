// Evaluation from C: eval, R_tryEval, R_ParseVector, options.
// R: main/eval.c:eval, main/context.c:R_tryEval, main/parse.c:R_ParseVector
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Parse.h>

SEXP capi_eval(void) {
    SEXP call = PROTECT(lang3(install("+"), ScalarInteger(20), ScalarInteger(22)));
    SEXP v = PROTECT(eval(call, R_GlobalEnv));
    SEXP e = PROTECT(eval(lang2(install("sqrt"), ScalarReal(49.0)), R_BaseEnv));
    SEXP ans = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(ans, 0, v);
    SET_VECTOR_ELT(ans, 1, e);
    UNPROTECT(4);
    return ans;
}

// R_tryEval catches errors: err is set, the value NULL.
SEXP capi_tryeval(void) {
    int err1 = 0, err2 = 0;
    SEXP ok = PROTECT(R_tryEval(lang2(install("sqrt"), ScalarInteger(4)),
                                 R_GlobalEnv, &err1));
    SEXP bad = PROTECT(R_tryEval(lang2(install("stop"), mkString("from R_tryEval")),
                                 R_GlobalEnv, &err2));
    SEXP ans = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 0, ok);
    SET_VECTOR_ELT(ans, 1, ScalarInteger(err1));
    SET_VECTOR_ELT(ans, 2, ScalarLogical(bad == NULL ? 1 : 0));
    UNPROTECT(3);
    return ans;
}

// Parse from C and evaluate the expressions.
SEXP capi_parse(void) {
    ParseStatus status;
    SEXP exprs = PROTECT(R_ParseVector(mkString("1 + 2 * 3\nsqrt(16)"), -1, &status, R_NilValue));
    SEXP ans = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 0, ScalarInteger(status));
    SET_VECTOR_ELT(ans, 1, eval(VECTOR_ELT(exprs, 0), R_GlobalEnv));
    SET_VECTOR_ELT(ans, 2, eval(VECTOR_ELT(exprs, 1), R_GlobalEnv));
    UNPROTECT(2);
    return ans;
}

// GetOption1 reads an option without evaluation.
SEXP capi_option(void) {
    return GetOption1(install("digits"));
}
