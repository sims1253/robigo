#include <R.h>
#include <Rinternals.h>
SEXP leak(void) { PROTECT(ScalarInteger(1)); return R_NilValue; }
