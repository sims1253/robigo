// C-level tryCatch: R_tryCatch, R_tryCatchError, R_withCallingErrorHandler.
// R: main/errors.c:R_tryCatch, R_tryCatchError, R_withCallingErrorHandler
#include <R.h>
#include <Rinternals.h>

// The body evaluates an R call in the global environment.
static SEXP tc_body_eval(void *data) {
    Rprintf("C body\n");
    return Rf_eval((SEXP) data, R_GlobalEnv);
}

// The body errors from C.
static SEXP tc_body_error(void *data) {
    Rprintf("C body\n");
    Rf_error("error from the C body: %s", (const char *) data);
    return R_NilValue;  // not reached
}

// The handler: the condition's class and message, then its own value.
static SEXP tc_handler(SEXP cond, void *data) {
    SEXP klass = Rf_getAttrib(cond, R_ClassSymbol);
    SEXP msg = VECTOR_ELT(cond, 0);
    Rprintf("C handler (%s): class %s, message %s\n", (const char *) data,
            CHAR(STRING_ELT(klass, 0)), CHAR(STRING_ELT(msg, 0)));
    return Rf_mkString("value of the C handler");
}

static void tc_finally(void *data) {
    Rprintf("C finally (%s)\n", (const char *) data);
}

// R_tryCatchError around R code
SEXP capi_tc_error(SEXP expr) {
    return R_tryCatchError(tc_body_eval, expr, tc_handler, "tryCatchError");
}

// R_tryCatchError around C that errors
SEXP capi_tc_error_c(void) {
    return R_tryCatchError(tc_body_error, "from C", tc_handler, "tryCatchError");
}

// R_tryCatch with classes and an optional finally
SEXP capi_tc(SEXP expr, SEXP classes, SEXP fin) {
    return R_tryCatch(tc_body_eval, expr, classes, tc_handler, "tryCatch",
                      LOGICAL(fin)[0] ? tc_finally : NULL, "tryCatch");
}

// R_tryCatch with no handler (the default returns NULL) and no conditions
SEXP capi_tc_defaults(SEXP expr) {
    return R_tryCatch(tc_body_eval, expr, NULL, NULL, NULL, NULL, NULL);
}

// R_tryCatch with no body
SEXP capi_tc_nobody(void) {
    return R_tryCatch(NULL, NULL, NULL, NULL, NULL, NULL, NULL);
}

// R_withCallingErrorHandler: the handler runs, then the error goes on
static SEXP wceh_handler(SEXP cond, void *data) {
    SEXP msg = VECTOR_ELT(cond, 0);
    Rprintf("C calling handler: %s\n", CHAR(STRING_ELT(msg, 0)));
    return R_NilValue;
}

SEXP capi_wceh(SEXP expr) {
    return R_withCallingErrorHandler(tc_body_eval, expr, wceh_handler, NULL);
}

// R_withCallingErrorHandler with no body
SEXP capi_wceh_nobody(void) {
    return R_withCallingErrorHandler(NULL, NULL, NULL, NULL);
}
