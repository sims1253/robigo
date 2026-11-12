/* A pairlist or call C builds onto an existing one keeps that one's cells
   as its tail: CDR(Rf_cons(car, x)) is x itself, so writing through the
   tail writes x (rlang's new_node() then node_cdr(), which tidyselect's
   node_compact_missing uses to drop the empty arguments of `c(, )`). */
#include <R.h>
#include <Rinternals.h>

SEXP capi_cons(SEXP car, SEXP x) { return Rf_cons(car, x); }
SEXP capi_lcons(SEXP car, SEXP x) { return Rf_lcons(car, x); }
SEXP capi_cdr(SEXP x) { return CDR(x); }
SEXP capi_setcar(SEXP x, SEXP v) { SETCAR(x, v); return R_NilValue; }
SEXP capi_setcdr(SEXP x, SEXP v) { SETCDR(x, v); return R_NilValue; }
SEXP capi_same(SEXP a, SEXP b) { return ScalarLogical(a == b); }
