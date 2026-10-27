// Console output: Rprintf/REprintf/Rvprintf, R_FlushConsole,
// R_CheckUserInterrupt.
// R: main/printutils.c:Rprintf, main/errors.c:R_CheckUserInterrupt
#include <R.h>
#include <Rinternals.h>
#include <stdarg.h>

void capi_vexample(const char *format, ...) {
    va_list ap;
    va_start(ap, format);
    Rvprintf(format, ap);
    va_end(ap);
}

void capi_vexample2(const char *format, ...) {
    va_list ap;
    va_start(ap, format);
    REvprintf(format, ap);  // to stderr
    va_end(ap);
}

SEXP capi_printf(void) {
    Rprintf("Rprintf: int %d, double %g, string %s\n", 42, 1.5, "s");
    REprintf("REprintf goes to stderr\n");
    capi_vexample("%s %d\n", "Rvprintf:", 7);
    capi_vexample2("%s %d\n", "REvprintf:", 8);
    R_FlushConsole();
    R_CheckUserInterrupt();  // no interrupt pending: returns quietly
    return mkString("done");
}

// PrintValue: C's print(); ExpandFileName; ProcessEvents.
// R: main/print.c:PrintValue, main/sysutils.c:R_ExpandFileName
void Rf_PrintDefaults(void);  // exported, not in public headers

SEXP capi_print(SEXP x) {
    PrintValue(x);
    Rf_PrintDefaults();  // the default print options, printed
    const char *home = R_ExpandFileName("~");
    Rprintf("home is %s\n", home);
    R_ProcessEvents();
    return mkString("printed");
}
