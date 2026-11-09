/* Rf_nlevels: the number of levels of a factor, 0 for anything else
   (R: Rinlinedfuns.h:nlevels). nanoparquet calls it when it writes a
   factor column. */
#include <R.h>
#include <Rinternals.h>

SEXP capi_nlevels(SEXP x)
{
    return ScalarInteger(Rf_nlevels(x));
}
