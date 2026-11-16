// An object's identity seen from C: its address, as GNU R's
// `.Internal(inspect())` prints it (tests/rt/capi/inplace-identity.R).
#include <stdio.h>
#include <R.h>
#include <Rinternals.h>

SEXP rt_addr(SEXP x) {
    char buf[32];
    snprintf(buf, sizeof buf, "%p", (void *)x);
    return mkString(buf);
}

// Whether evaluating the assignment `expr` in `rho` binds `sym` to a new
// object (the write copied) rather than the one bound before (it wrote in
// place). The old object stays on the protect stack, which counts nothing,
// so both objects are alive when they are compared. Comparing an address
// read before the write with one read after cannot tell a copy under a
// moving collector (ROBIGO_GC_MOVE=1): the old object dies at the write
// and a collection can move the copy to its address (#709).
SEXP rt_assign_copies(SEXP sym, SEXP expr, SEXP rho) {
    SEXP old = PROTECT(eval(sym, rho));
    eval(expr, rho);
    SEXP now = eval(sym, rho);
    UNPROTECT(1);
    return ScalarLogical(now != old);
}
