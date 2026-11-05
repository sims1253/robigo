/* A loop of lookups: GNU R hands out the bound object every time, so
   looking a value up does not cost memory. */
#include <R.h>
#include <Rinternals.h>

SEXP capi_loop_get(SEXP env, SEXP sym) { return findVarInFrame(env, sym); }
SEXP capi_loop_getvar(SEXP env, SEXP sym) { return R_getVar(sym, env, FALSE); }
