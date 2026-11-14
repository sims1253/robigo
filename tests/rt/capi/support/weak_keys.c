/* tests/rt/capi/weak-keys-collections.R's C: the C that
 * tests/r/capi/weak-keys-collections.R writes and compiles itself, checked
 * in (the foreman's answer 3: that script's tempfile, writeLines and
 * system2 join robigo-rt at M3), built against GNU R's headers. */
#include <R.h>
#include <Rinternals.h>

static SEXP held;
SEXP weak(SEXP k, SEXP v, SEXP f) { return R_MakeWeakRef(k, v, f, 0); }
SEXP key(SEXP w) { return R_WeakRefKey(w); }
SEXP touch(SEXP x) { return R_NilValue; }
static void fin(SEXP p) { Rprintf("C pointer finalized\n"); }
SEXP pointer(void) {
    SEXP p = R_MakeExternalPtr(0, R_NilValue, R_NilValue);
    R_RegisterCFinalizer(p, fin);
    return R_NilValue;
}
SEXP hold(SEXP x) { held = x; R_PreserveObject(x); return R_NilValue; }
SEXP release(void) { R_ReleaseObject(held); held = 0; return R_NilValue; }
/* garbage only C allocates: enough for several collections */
SEXP churn(SEXP n) {
    int k = Rf_asInteger(n);
    for (int i = 0; i < k; i++) Rf_allocVector(14, 100000);
    return R_NilValue;
}
