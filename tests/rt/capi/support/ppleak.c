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

/* The cell at `path` (1-based, as `[[` counts a call's elements) inside
   closure f's body set to e, in place: each index but the last steps into
   that element (a call), the last names the cell written. */
SEXP pp_set_path(SEXP f, SEXP path, SEXP e)
{
    SEXP x = R_ClosureExpr(f);
    int n = LENGTH(path);
    for (int k = 0; k < n; k++) {
        SEXP c = x;
        for (int j = INTEGER(path)[k]; j > 1; j--)
            c = CDR(c);
        if (k == n - 1)
            SETCAR(c, e);
        else
            x = CAR(c);
    }
    return R_NilValue;
}
