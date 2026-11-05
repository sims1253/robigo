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

/* A `...` C binds in an environment stays the chain it bound, through a
 * collection (the binding keeps it, as in GNU R). */
static SEXP bound = NULL;

SEXP capi_dots_bind(SEXP from, SEXP to) {
    bound = findVarInFrame(from, R_DotsSymbol);
    defineVar(install("d"), bound, to);
    return R_NilValue;
}

SEXP capi_dots_bound_same(SEXP to) {
    return ScalarLogical(findVarInFrame(to, install("d")) == bound);
}

/* A `...` lengthened with a LANGSXP cell (SETCDR of its last cell). */
SEXP capi_dots_append_lang(SEXP list) {
    SEXP d = VECTOR_ELT(list, 0);
    while (CDR(d) != R_NilValue)
        d = CDR(d);
    SETCDR(d, LCONS(ScalarInteger(3), R_NilValue));
    return R_NilValue;
}
