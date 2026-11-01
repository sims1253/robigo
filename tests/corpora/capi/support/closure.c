// A closure assembled in C: allocSExp(CLOSXP) with SET_FORMALS,
// SET_BODY and SET_CLOENV, as stats' deriv builds its gradient
// function (src/library/stats/src/deriv.c).
// R: main/memory.c:allocSExp, Rinternals.h:SET_FORMALS/SET_BODY/SET_CLOENV
#include <R.h>
#include <Rinternals.h>

SEXP capi_cclosure(void) {
    SEXP f = PROTECT(allocList(2));
    SET_TAG(f, install("x"));
    SETCAR(f, R_MissingArg);
    SET_TAG(CDR(f), install("y"));
    SETCAR(CDR(f), ScalarReal(2));

    SEXP body = PROTECT(lang3(install("*"),
                             install("x"),
                             lang3(install("+"), install("y"), ScalarInteger(1))));

    SEXP clo = PROTECT(allocSExp(CLOSXP));
    SET_FORMALS(clo, f);
    SET_BODY(clo, body);
    SET_CLOENV(clo, R_GlobalEnv);
    UNPROTECT(3);
    return clo;
}
