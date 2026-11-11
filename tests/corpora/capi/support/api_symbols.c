// Symbols Rinternals.h exports as data: each must be the installed symbol
// of its name.
#include <R.h>
#include <Rinternals.h>

SEXP capi_api_symbols(void) {
    SEXP syms[] = {R_AtsignSymbol, R_NamespaceEnvSymbol};
    int n = sizeof(syms) / sizeof(syms[0]);
    SEXP ans = PROTECT(allocVector(STRSXP, n));
    for (int i = 0; i < n; i++) {
        if (syms[i] != install(CHAR(PRINTNAME(syms[i]))))
            error("symbol %d is not the installed one", i);
        SET_STRING_ELT(ans, i, PRINTNAME(syms[i]));
    }
    UNPROTECT(1);
    return ans;
}
