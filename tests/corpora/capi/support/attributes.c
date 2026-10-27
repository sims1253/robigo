// Attributes: getAttrib/setAttrib, the predefined attribute symbols.
// R: main/attrib.c
#include <R.h>
#include <Rinternals.h>
#include <R_ext/GraphicsEngine.h>  // col2name, ANY_ATTRIB

// Reading attributes, including the NULL of an absent one.
SEXP capi_attrib_get(SEXP x) {
    SEXP ans = PROTECT(allocVector(VECSXP, 6));
    SET_VECTOR_ELT(ans, 0, getAttrib(x, R_DimSymbol));
    SET_VECTOR_ELT(ans, 1, getAttrib(x, R_DimNamesSymbol));
    SET_VECTOR_ELT(ans, 2, getAttrib(x, R_NamesSymbol));
    SET_VECTOR_ELT(ans, 3, getAttrib(x, R_ClassSymbol));
    SET_VECTOR_ELT(ans, 4, getAttrib(x, R_RowNamesSymbol));
    SET_VECTOR_ELT(ans, 5, getAttrib(x, R_LevelsSymbol));
    UNPROTECT(1);
    return ans;
}

// Setting attributes on a duplicate; namesgets and classgets are the
// C level names<- and class<-.
SEXP capi_attrib_set(SEXP x, SEXP names) {
    SEXP y = PROTECT(duplicate(x));
    setAttrib(y, R_NamesSymbol, names);
    namesgets(y, names);           // same effect as setAttrib for names
    SEXP dim = PROTECT(allocVector(INTSXP, 2));
    INTEGER(dim)[0] = 2;
    INTEGER(dim)[1] = 3;
    setAttrib(y, R_DimSymbol, dim);
    SEXP cls = PROTECT(allocVector(STRSXP, 1));
    SET_STRING_ELT(cls, 0, mkChar("capi_cls"));
    classgets(y, cls);
    SEXP rn = PROTECT(allocVector(INTSXP, 2));
    INTEGER(rn)[0] = 101;
    INTEGER(rn)[1] = 102;
    setAttrib(y, R_RowNamesSymbol, rn);
    UNPROTECT(4);
    return y;
}

// The attribute list itself: ATTRIB is a pairlist; SET_ATTRIB replaces it
// (cleared with R_NilValue); copyMostAttrib copies all but names/dim/dimnames.
SEXP capi_attrib_list(SEXP from) {
    SEXP to = PROTECT(allocVector(REALSXP, 4));
    copyMostAttrib(from, to);
    int nattr = length(ATTRIB(to));
    SEXP cls = getAttrib(to, R_ClassSymbol);  // copied by copyMostAttrib
    SET_ATTRIB(to, R_NilValue);
    SEXP ans = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 0, ScalarInteger(nattr));
    SET_VECTOR_ELT(ans, 1, cls);
    SET_VECTOR_ELT(ans, 2, ScalarInteger(length(ATTRIB(to))));  // 0 now
    UNPROTECT(2);
    return ans;
}

// inherits and isObject.
SEXP capi_inherits(SEXP x) {
    SEXP ans = PROTECT(allocVector(LGLSXP, 4));
    LOGICAL(ans)[0] = inherits(x, "data.frame");
    LOGICAL(ans)[1] = isObject(x);
    LOGICAL(ans)[2] = isS4(x);
    LOGICAL(ans)[3] = inherits(getAttrib(x, R_ClassSymbol), "character");
    UNPROTECT(1);
    return ans;
}

// The duplication family: duplicate, shallow_duplicate, and the
// attribute-only variants.
// R: main/duplicate.c, main/attrib.c:DUPLICATE_ATTRIB
SEXP capi_duplicate(SEXP x) {
    SEXP d = PROTECT(duplicate(x));
    SEXP s = PROTECT(shallow_duplicate(x));
    SEXP to = PROTECT(allocVector(REALSXP, 4));
    SHALLOW_DUPLICATE_ATTRIB(x, to);
    SEXP to2 = PROTECT(allocVector(REALSXP, 4));
    DUPLICATE_ATTRIB(to2, x);
    SEXP z = PROTECT(allocVector(REALSXP, 2));
    CLEAR_ATTRIB(z);
    int any = ANY_ATTRIB(x);
    SEXP ans = PROTECT(allocVector(VECSXP, 6));
    SET_VECTOR_ELT(ans, 0, d);
    SET_VECTOR_ELT(ans, 1, s);
    SET_VECTOR_ELT(ans, 2, ScalarInteger(length(ATTRIB(to))));
    SET_VECTOR_ELT(ans, 3, ScalarInteger(length(ATTRIB(to2))));
    SET_VECTOR_ELT(ans, 4, ScalarLogical(any));
    SET_VECTOR_ELT(ans, 5, ScalarInteger(length(ATTRIB(z))));
    UNPROTECT(7);
    return ans;
}

// col2name names colour numbers; the palette lives in grDevices, so this
// errors when it is not loaded.
SEXP capi_col2name(void) {
    return mkString(col2name((rcolor)2));
}

// dimnamesgets and col2name (the C level dimnames<- and colour naming).
// R: main/attrib.c:dimnamesgets, grDevices colors.c:Rf_col2name
SEXP capi_dimnames(SEXP x) {
    SEXP y = PROTECT(duplicate(x));
    SEXP rn = PROTECT(allocVector(STRSXP, 2));
    SET_STRING_ELT(rn, 0, mkChar("r1"));
    SET_STRING_ELT(rn, 1, mkChar("r2"));
    SEXP cn = PROTECT(allocVector(STRSXP, 2));
    SET_STRING_ELT(cn, 0, mkChar("c1"));
    SET_STRING_ELT(cn, 1, mkChar("c2"));
    SEXP dn = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(dn, 0, rn);
    SET_VECTOR_ELT(dn, 1, cn);
    dimnamesgets(y, dn);
    SEXP ans = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(ans, 0, y);
    SET_VECTOR_ELT(ans, 1, ScalarLogical(TRUE));
    UNPROTECT(6);
    return ans;
}
