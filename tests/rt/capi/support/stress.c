/* Experiment A's stress script's C (docs/g3.md §15.1, docs/ledger/core.md
 * 2026-11-11; tests/rt/stress/finalizer-depths.R): an external pointer with
 * a C finalizer, made and dropped by one .Call, and C code that only
 * allocates, so that its collections find the pointer dead. */
#include <R.h>
#include <Rinternals.h>
#include <stdint.h>

#define N 2048
static int done[N];

static void fin(SEXP p)
{
    intptr_t k = (intptr_t) R_ExternalPtrAddr(p) - 1;
    if (k >= 0 && k < N)
        done[k]++;
}

/* an external pointer for id, finalized by fin, held by nothing on return */
SEXP pointer(SEXP id)
{
    int k = asInteger(id);
    SEXP p = PROTECT(R_MakeExternalPtr((void *) (intptr_t) (k + 1), R_NilValue, R_NilValue));
    R_RegisterCFinalizer(p, fin);
    UNPROTECT(1);
    return R_NilValue;
}

/* garbage only C allocates: enough for several collections */
SEXP churn(SEXP n)
{
    int k = asInteger(n);
    for (int i = 0; i < k; i++)
        allocVector(REALSXP, 100000);
    return R_NilValue;
}

/* how many times id's finalizer ran */
SEXP finalized(SEXP id)
{
    return ScalarInteger(done[asInteger(id)]);
}
