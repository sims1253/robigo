/* The routines of tests/rt/capi/ppacct-*.R: one that leaves an object
   protected, as a package with a missing UNPROTECT does, for eval.c's
   "stack imbalance" warnings at each evaluator site that checks the
   protect stack; and a balanced one that does nothing. */
#include <R.h>
#include <Rinternals.h>

SEXP leak(void)
{
    PROTECT(ScalarInteger(1));
    return R_NilValue;
}

SEXP noop(void)
{
    return R_NilValue;
}
