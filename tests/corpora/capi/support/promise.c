// Promise expressions: R_PromiseExpr returns the expression behind a
// promise (the bytecode-free source).
// R: main/eval.c:R_PromiseExpr
#include <R.h>
#include <Rinternals.h>

SEXP capi_promise_expr(SEXP env, SEXP name) {
    SEXP p = findVarInFrame(env, name);
    return R_PromiseExpr(p);
}
