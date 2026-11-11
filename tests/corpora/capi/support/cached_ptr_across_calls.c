/* C keeps a raw INTEGER() pointer in a static across .Calls and writes
 * through it in a later call without taking it again. GNU R's pointer is
 * the vector's own, so R sees the write. robigo compares a small copy
 * twin C took a writable pointer of only until the call that took it
 * returns (bridge.rs SMALL_CACHED): this later write is a known gap. */
#include <R.h>
#include <Rinternals.h>

static int *cached;

/* a fresh vector C made (a copy twin once R holds it) */
SEXP capi_fresh_pair(void) {
    SEXP x = PROTECT(Rf_allocVector(INTSXP, 2));
    INTEGER(x)[0] = 1;
    INTEGER(x)[1] = 2;
    UNPROTECT(1);
    return x;
}

SEXP capi_cache_ptr(SEXP x) {
    cached = INTEGER(x);
    return R_NilValue;
}

SEXP capi_write_cached(SEXP v) {
    if (!cached)
        error("capi_write_cached: no pointer cached");
    cached[0] = Rf_asInteger(v);
    return R_NilValue;
}
