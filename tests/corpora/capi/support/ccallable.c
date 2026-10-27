// R_GetCCallable: fetch what ccallable_reg.so published and call it.
// R: main/Rdynload.c:R_GetCCallable
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Rdynload.h>

static double (*capi_add)(double, double) = NULL;
static int (*capi_gcd)(int, int) = NULL;

static void capi_fetch(void) {
    if (capi_add == NULL) {
        capi_add = (double (*)(double, double))R_GetCCallable("capi_ccallables",
                                                              "capi_ccall_add");
        capi_gcd = (int (*)(int, int))R_GetCCallable("capi_ccallables",
                                                     "capi_ccall_gcd");
    }
}

SEXP capi_use_ccallable(void) {
    capi_fetch();
    return ScalarReal(capi_add(1.5, 2.25));
}

SEXP capi_use_gcd(void) {
    capi_fetch();
    return ScalarInteger(capi_gcd(252, 105));
}
