// Routine registration: R_registerRoutines with the C, Call and External
// tables, R_useDynamicSymbols(dll, FALSE), R_forceSymbols(dll, TRUE).
// R: main/Rdynload.c, WRE "Registering native routines"
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Rdynload.h>

static SEXP c_add(SEXP a, SEXP b) {
    return ScalarInteger(asInteger(a) + asInteger(b));
}

// .C: plain pointers, the old interface.
static void c_sum2(int *x, int *n, int *out) {
    int s = 0;
    for (int i = 0; i < *n; i++) s += x[i];
    *out = s;
}

// .External: one pairlist of the arguments, with the routine name first.
static SEXP c_first(SEXP args) {
    return CADR(args);
}

// .External with a name in front: the classic "NAME(obj)" style.
static SEXP c_count_args(SEXP args) {
    return ScalarInteger(length(args) - 1);
}

static const R_CMethodDef CEntries[] = {
    {"c_sum2", (DL_FUNC)&c_sum2, 3},
    {NULL, NULL, 0}};

static const R_CallMethodDef CallEntries[] = {
    {"c_add", (DL_FUNC)&c_add, 2},
    {NULL, NULL, 0}};

static const R_ExternalMethodDef ExtEntries[] = {
    {"c_first", (DL_FUNC)&c_first, 2},
    {"c_count_args", (DL_FUNC)&c_count_args, -1},
    {NULL, NULL, 0}};

void R_init_registration(DllInfo *dll) {
    R_registerRoutines(dll, CEntries, CallEntries, NULL, ExtEntries);
    R_useDynamicSymbols(dll, FALSE);
}
