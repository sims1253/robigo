/* rlang's quo_squash() (sims1253/robigo#432): R code walks a duplicated
   call with rlang's node accessors, each a .Call of its own, and pokes
   every quosure's expression into the cell that held it. The cell is a
   tail C handed back earlier (node_cdr), and the call is one C copied
   (duplicate), so the write must reach that copy through the tail. */
#include <R.h>
#include <Rinternals.h>

SEXP capi_duplicate(SEXP x) { return Rf_duplicate(x); }
SEXP capi_node_car(SEXP x) { return CAR(x); }
SEXP capi_node_cdr(SEXP x) { return CDR(x); }
SEXP capi_node_cadr(SEXP x) { return CADR(x); }
/* rlang's ffi_node_poke_car returns the node it wrote into */
SEXP capi_node_poke_car(SEXP x, SEXP v) { SETCAR(x, v); return x; }
