// R_forceSymbols(dll, TRUE): the DLL refuses calls by routine name; the R
// side must pass native symbol objects (getNativeSymbolInfo).
// R: main/Rdynload.c:R_forceSymbols, main/dotcode.c:R_cdynload
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Rdynload.h>

static SEXP c_mul(SEXP a, SEXP b) {
    return ScalarInteger(asInteger(a) * asInteger(b));
}

static const R_CallMethodDef CallEntries[] = {
    {"c_mul", (DL_FUNC)&c_mul, 2},
    {NULL, NULL, 0}};

void R_init_registration2(DllInfo *dll) {
    R_registerRoutines(dll, NULL, CallEntries, NULL, NULL);
    R_useDynamicSymbols(dll, FALSE);
    R_forceSymbols(dll, TRUE);
}
