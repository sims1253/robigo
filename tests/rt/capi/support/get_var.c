/* R_getVar and R_getVarEx, the C API's (envir.c): a symbol's value in an
 * environment (and its enclosures with inherits), a promise forced; an
 * unbound one is R_getVar's error or R_getVarEx's ifnotfound, a missing
 * argument the missingArgError of subclass getVarExError. */
#include <R.h>
#include <Rinternals.h>

SEXP rt_get_var(SEXP sym, SEXP rho, SEXP inherits)
{
    return R_getVar(sym, rho, (Rboolean) asLogical(inherits));
}

SEXP rt_get_var_ex(SEXP sym, SEXP rho, SEXP inherits, SEXP ifnotfound)
{
    return R_getVarEx(sym, rho, (Rboolean) asLogical(inherits), ifnotfound);
}
