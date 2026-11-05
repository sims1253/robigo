/* C writes through cells it reached from an object R holds: a pairlist's
   car (rlang's deparser keeps its indentation state that way:
   node_poke_cadr(node_car(self$indent_status), n)), and a list inside a
   list. GNU R writes the object itself, so R sees the change. */
#include <R.h>
#include <Rinternals.h>

SEXP capi_poke_car(SEXP x) { return CAR(x); }
SEXP capi_poke_cadr(SEXP x) { return CADR(x); }
SEXP capi_poke_set_cadr(SEXP x, SEXP v) { SETCADR(x, v); return R_NilValue; }
SEXP capi_poke_set_car(SEXP x, SEXP v) { SETCAR(x, v); return R_NilValue; }
SEXP capi_poke_elt(SEXP x, SEXP i) { return VECTOR_ELT(x, INTEGER(i)[0]); }
SEXP capi_poke_set_elt(SEXP x, SEXP i, SEXP v)
{
    SET_VECTOR_ELT(x, INTEGER(i)[0], v);
    return R_NilValue;
}

/* through a lookup: the variable's list, its first element's second slot */
SEXP capi_poke_lookup(SEXP env, SEXP v)
{
    SEXP x = findVarInFrame(env, install("state"));
    SET_VECTOR_ELT(VECTOR_ELT(x, 0), 1, v);
    return R_NilValue;
}
