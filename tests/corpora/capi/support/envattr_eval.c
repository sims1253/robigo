// Attributes C sets on an environment are the environment's at once: R
// code the routine evaluates before it returns sees them (rlang's
// eval_tidy() classes its `.env` pronoun, then evaluates the quosure in
// the data mask, all in one .External2 call).
// R: attrib.c:setAttrib, memory.c:SET_S4_OBJECT, eval.c:eval
#include <R.h>
#include <Rinternals.h>

SEXP capi_mask_eval(SEXP expr) {
    SEXP mask = PROTECT(R_NewEnv(R_GlobalEnv, FALSE, 10));
    SEXP pron = PROTECT(R_NewEnv(R_GlobalEnv, FALSE, 0));
    setAttrib(pron, R_ClassSymbol, mkString("pronoun"));
    defineVar(install(".env"), pron, mask);
    SEXP out = eval(expr, mask);
    UNPROTECT(2);
    return out;
}

SEXP capi_attr_eval(SEXP env, SEXP expr) {
    setAttrib(env, install("tag"), mkString("set in C"));
    return eval(expr, R_GlobalEnv);
}

SEXP capi_unclass_eval(SEXP env, SEXP expr) {
    setAttrib(env, R_ClassSymbol, R_NilValue);
    return eval(expr, R_GlobalEnv);
}

SEXP capi_s4_eval(SEXP env, SEXP expr) {
    SET_S4_OBJECT(env);
    return eval(expr, R_GlobalEnv);
}
