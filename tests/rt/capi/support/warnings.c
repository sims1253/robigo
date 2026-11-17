/* errors.c's warning() removes one trailing newline of its message;
 * warningcall() keeps it (tests/rt/capi/warning-newline.R). */
#include <R.h>
#include <Rinternals.h>

SEXP capi_warning(SEXP msg) {
    Rf_warning("%s", CHAR(STRING_ELT(msg, 0)));
    return R_NilValue;
}

SEXP capi_warningcall(SEXP msg) {
    SEXP call = PROTECT(Rf_lang1(Rf_install("caller")));
    Rf_warningcall(call, "%s", CHAR(STRING_ELT(msg, 0)));
    UNPROTECT(1);
    return R_NilValue;
}
