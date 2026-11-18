/* Rf_asBool, the C API's (coerce.c:asBool): asLogical2(x, 1, R_NilValue),
 * so a long atomic vector is an error with no call, and NA is error()'s,
 * against the context call. */
#include <R.h>
#include <Rinternals.h>

SEXP rt_as_bool(SEXP x)
{
    return ScalarLogical(Rf_asBool(x));
}
