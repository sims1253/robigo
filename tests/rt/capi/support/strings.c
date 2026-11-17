// C compares CHARSXPs by pointer: GNU R caches every one, so equal strings
// are one object, the one mkChar of their bytes returns, and a symbol's
// PRINTNAME. Each element is read first, in its own statement.
#include <R.h>
#include <Rinternals.h>

SEXP rt_elt_is_mkchar(SEXP x, SEXP i) {
    SEXP e = STRING_ELT(x, asInteger(i));
    SEXP m = mkChar(CHAR(e));
    return ScalarLogical(e == m);
}

SEXP rt_elt_is_printname(SEXP x, SEXP i) {
    SEXP e = STRING_ELT(x, asInteger(i));
    SEXP s = install(CHAR(e));
    return ScalarLogical(PRINTNAME(s) == e);
}

SEXP rt_elts_same(SEXP x, SEXP i, SEXP y, SEXP j) {
    SEXP a = STRING_ELT(x, asInteger(i));
    SEXP b = STRING_ELT(y, asInteger(j));
    return ScalarLogical(a == b);
}
