// R_RegisterCCallable: this library publishes a C entry point for other
// libraries (not R code) to fetch with R_GetCCallable.
// R: main/Rdynload.c:R_RegisterCCallable
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Rdynload.h>

static double capi_ccall_add(double a, double b) { return a + b; }

static int capi_ccall_gcd(int a, int b) {
    while (b != 0) {
        int t = b;
        b = a % b;
        a = t;
    }
    return a;
}

void R_init_ccallable_reg(DllInfo *dll) {
    R_RegisterCCallable("capi_ccallables", "capi_ccall_add", (DL_FUNC)&capi_ccall_add);
    R_RegisterCCallable("capi_ccallables", "capi_ccall_gcd", (DL_FUNC)&capi_ccall_gcd);
}

// The provider uses its own callable too.
SEXP capi_self(void) {
    double (*add)(double, double) =
        (double (*)(double, double))R_GetCCallable("capi_ccallables", "capi_ccall_add");
    return ScalarReal(add(1.0, 2.0));
}
