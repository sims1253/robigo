/* rlang's new_binary_closure() (R/fn.R, rlang 1.3.0) builds an operator's
   closure body from C nodes: a shallow duplicate of a pairlist, a SETCDR
   from inside a callback closure on its last node, and a call made with
   Rf_lcons. Each accessor is a .Call of its own, as in rlang. */
#include <R.h>
#include <Rinternals.h>

SEXP capi_duplicate(SEXP x, SEXP shallow) {
    return Rf_asLogical(shallow) ? Rf_shallow_duplicate(x) : Rf_duplicate(x);
}
SEXP capi_node_cdr(SEXP x) { return CDR(x); }
SEXP capi_node_poke_cdr(SEXP x, SEXP y) { SETCDR(x, y); return R_NilValue; }
SEXP capi_new_call(SEXP car, SEXP cdr) { return Rf_lcons(car, cdr); }
