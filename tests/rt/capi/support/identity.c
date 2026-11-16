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
