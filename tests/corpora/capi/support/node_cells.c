/* A call's cells are its own (sims1253/robigo#387): CDR(x) is a cell of
   x, so writing through a tail C took writes the call, two CDRs of a call
   are one object, and splicing cells in through a tail grows the call
   (tidyselect's call_expand_dots, node_poke_tail). */
#include <R.h>
#include <Rinternals.h>

SEXP capi_node_cdr(SEXP x) { return CDR(x); }
SEXP capi_node_cddr(SEXP x) { return CDDR(x); }
SEXP capi_node_poke_car(SEXP x, SEXP v) { SETCAR(x, v); return R_NilValue; }
SEXP capi_node_poke_cdr(SEXP x, SEXP v) { SETCDR(x, v); return R_NilValue; }
SEXP capi_node_same(SEXP a, SEXP b) { return ScalarLogical(a == b); }
