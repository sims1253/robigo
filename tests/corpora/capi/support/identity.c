/* Core-object identity across calls (#278): GNU R passes the caller's own
   objects to C (dotcode.c) and never moves them, so an object's address is
   the same in every call while R holds it. data.table's .onLoad checks this
   with address(), and C keeps objects it created or was given (preserved)
   and compares what R hands back. */
#include <R.h>
#include <Rinternals.h>
#include <stdio.h>

SEXP capi_id_addr(SEXP x)
{
    char buf[64];
    snprintf(buf, sizeof buf, "%p", (void *) x);
    return mkString(buf);
}

/* the columns of two lists in one list (a C cbind) */
SEXP capi_id_cbind(SEXP a, SEXP b)
{
    R_xlen_t na = XLENGTH(a), nb = XLENGTH(b);
    SEXP out = PROTECT(allocVector(VECSXP, na + nb));
    for (R_xlen_t i = 0; i < na; i++)
        SET_VECTOR_ELT(out, i, VECTOR_ELT(a, i));
    for (R_xlen_t i = 0; i < nb; i++)
        SET_VECTOR_ELT(out, na + i, VECTOR_ELT(b, i));
    UNPROTECT(1);
    return out;
}

static SEXP kept = NULL;

static void keep(SEXP x)
{
    if (kept != NULL)
        R_ReleaseObject(kept);
    kept = x;
    R_PreserveObject(x);
}

/* an object C creates, keeps and returns */
SEXP capi_id_make_keep(SEXP type)
{
    SEXP x;
    if (INTEGER(type)[0] == 0) {
        x = PROTECT(allocVector(REALSXP, 2));
        REAL(x)[0] = 1;
        REAL(x)[1] = 2;
    } else {
        x = PROTECT(allocVector(VECSXP, 2));
        SET_VECTOR_ELT(x, 0, ScalarInteger(1));
        SET_VECTOR_ELT(x, 1, mkString("a"));
    }
    keep(x);
    UNPROTECT(1);
    return x;
}

/* an argument (a temporary in R) C keeps and returns */
SEXP capi_id_keep_arg(SEXP x)
{
    keep(x);
    return x;
}

SEXP capi_id_is_kept(SEXP x) { return ScalarLogical(x == kept); }
