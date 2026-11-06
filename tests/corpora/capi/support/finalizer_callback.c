// An object that dies in R code C called back into (rlang's eval_bare
// under testthat's expect_output): gc() there finalizes it, as in GNU R.
// R: memory.c marks precisely; robigo's conservative stack scan covers
// only C frames, not stale words of the runtime's frames below them.
#include <R.h>
#include <Rinternals.h>

// Pass an object through C (its proxy's address lands on the stack).
SEXP capi_fc_touch(SEXP x) {
    return x;
}

// Evaluate R code from C.
SEXP capi_fc_eval(SEXP expr, SEXP env) {
    return eval(expr, env);
}
