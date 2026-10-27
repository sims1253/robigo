// Errors and warnings: Rf_error/errorcall end the computation (and the
// script), Rf_warning/warningcall are recoverable.
// R: main/errors.c
#include <R.h>
#include <Rinternals.h>

// Not in public headers.
void Rf_onintr(void);
void R_BadLongVector(SEXP x, const char *file, int line);
void R_signal_protect_error(void);
void R_signal_reprotect_error(void);

SEXP capi_warn(SEXP call) {
    warning("a plain warning, %d of %d", 1, 2);
    warningcall(call, "a warning reported against the call");
    warningcall_immediate(call, "an immediate warning");
    return ScalarInteger(1);
}

SEXP capi_err(void) {
    error("fatal: %s (%d)", "the end", 42);  // does not return
    return R_NilValue;                        // not reached
}

SEXP capi_errcall(SEXP call) {
    errorcall(call, "fatal via errorcall: %g", 1.5);
    return R_NilValue;
}

// The catchable fatal paths (from R, wrap in tryCatch).
SEXP capi_badlongvector(void) {
    R_BadLongVector(R_NilValue, "capi.c", 42);  // errors
    return R_NilValue;
}

SEXP capi_protect_overflow(void) {
    R_signal_protect_error();  // errors: protect(): protection stack overflow
    return R_NilValue;
}

SEXP capi_reprotect_error(void) {
    R_signal_reprotect_error();  // errors on REPROTECT misuse
    return R_NilValue;
}

// Rf_onintr with no interrupt pending jumps to top level, ending the
// script like Ctrl-C would: must be the last call of its case.
SEXP capi_onintr(void) {
    Rprintf("before onintr\n");
    Rf_onintr();
    Rprintf("after onintr (not reached)\n");
    return R_NilValue;
}
