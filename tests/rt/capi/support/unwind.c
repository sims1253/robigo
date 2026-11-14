// Non-local exits from C: R_UnwindProtect / R_ContinueUnwind /
// R_MakeUnwindCont / R_ExecWithCleanup / R_ToplevelExec.
// R: main/context.c:R_UnwindProtect, R_ExecWithCleanup
#include <R.h>
#include <Rinternals.h>

static SEXP capi_boom_body(void *data) {
    Rprintf("body: about to error\n");
    error("boom from the body");  // jumps into the unwind protect
    return R_NilValue;             // not reached
}

static void capi_cleanup(void *data, Rboolean jump) {
    Rprintf("cleanup: jump = %d\n", jump);
}

// The body errors; the cleanup runs with jump = TRUE; R_ContinueUnwind
// rethrows into the R-level handler that called us.
SEXP capi_unwind(void) {
    SEXP cont = PROTECT(R_MakeUnwindCont());
    SEXP token = R_UnwindProtect(capi_boom_body, NULL, capi_cleanup, NULL, cont);
    if (token == cont) {
        UNPROTECT(1);
        R_ContinueUnwind(cont);  // does not return
    }
    UNPROTECT(1);
    return token;
}

// The same, with the body returning normally: cleanup runs with jump=FALSE.
static SEXP capi_ok_body(void *data) {
    Rprintf("ok body\n");
    return ScalarInteger((int)(size_t)data);
}

SEXP capi_unwind_ok(void) {
    SEXP cont = PROTECT(R_MakeUnwindCont());
    SEXP token = R_UnwindProtect(capi_ok_body, (void *)41, capi_cleanup, NULL, cont);
    Rboolean unwound = token == cont;
    UNPROTECT(1);
    return unwound ? mkString("<unwound>") : token;
}

// R_ExecWithCleanup: always runs the cleanup, void* flavour.
static SEXP capi_ewc_body(void *data) {
    Rprintf("ewc body\n");
    return ScalarInteger(5);
}

static void capi_ewc_cleanup(void *data) {
    Rprintf("ewc cleanup\n");
}

SEXP capi_execcleanup(void) {
    return R_ExecWithCleanup(capi_ewc_body, NULL, capi_ewc_cleanup, NULL);
}

// R_ToplevelExec runs fun() where an error would jump to top level (and
// so end the script); here it simply returns.
static void capi_toplevel_body(void *data) {
    Rprintf("toplevel body\n");
}

SEXP capi_toplevelexec(void) {
    return ScalarLogical(R_ToplevelExec(capi_toplevel_body, NULL));
}
