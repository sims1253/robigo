/* `...` through native code: a DOTSXP C reads, passes on or returns is
 * still `...` on the R side (waldo's compare() of closures whose frames
 * hold unforced dots relies on it). */
#include <R.h>
#include <Rinternals.h>

/* Reads the list's names, as rlang's names2() does. */
SEXP capi_dots_names(SEXP x) {
    return getAttrib(x, R_NamesSymbol);
}

/* The `...` binding of a frame, as findVarInFrame gives it. */
SEXP capi_dots_get(SEXP env) {
    return findVarInFrame(env, R_DotsSymbol);
}

/* The element itself, back through the API. */
SEXP capi_dots_elt(SEXP x) {
    return VECTOR_ELT(x, 0);
}

SEXP capi_dots_type(SEXP x) {
    return mkString(type2char(TYPEOF(x)));
}
