/* C writes into the cells of a `...` (SETCAR, SET_TAG on the DOTSXP chain):
 * the frame that binds the `...` sees them, as in GNU R, where the chain C
 * holds is the frame's own. */
#include <R.h>
#include <Rinternals.h>

/* The frame's `...`, found and written in place. */
SEXP capi_dots_write_frame(SEXP env) {
    SEXP d = findVarInFrame(env, R_DotsSymbol);
    SETCAR(d, ScalarInteger(99));
    SET_TAG(CDR(d), install("b"));
    return R_NilValue;
}

/* A `...` handed over as an argument, written in place. */
SEXP capi_dots_write_arg(SEXP d) {
    SETCAR(d, mkString("written"));
    return R_NilValue;
}

/* Found twice in one call: the same chain. */
SEXP capi_dots_same(SEXP env) {
    return ScalarLogical(findVarInFrame(env, R_DotsSymbol) == findVarInFrame(env, R_DotsSymbol));
}
