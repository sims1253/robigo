/* rlang's node accessors: each a .Call of its own, so every node C hands
   back crosses the bridge and is handed to C again later. */
#include <R.h>
#include <Rinternals.h>

SEXP capi_new_node(SEXP car) { return Rf_cons(car, R_NilValue); }
SEXP capi_node_cdr(SEXP x) { return CDR(x); }
SEXP capi_node_poke_cdr(SEXP x, SEXP y) { SETCDR(x, y); return R_NilValue; }
