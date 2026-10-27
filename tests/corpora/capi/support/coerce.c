// Coercion and type predicates.
// R: main/coerce.c, main/util.c:type2char
#include <R.h>
#include <Rinternals.h>

// coerceVector between the atomic types.
SEXP capi_coerce(SEXP x) {
    SEXP i = PROTECT(coerceVector(x, INTSXP));
    SEXP d = PROTECT(coerceVector(x, REALSXP));
    SEXP l = PROTECT(coerceVector(x, LGLSXP));
    SEXP s = PROTECT(coerceVector(x, STRSXP));
    SEXP c = PROTECT(coerceVector(x, CPLXSXP));
    SEXP ans = PROTECT(allocVector(VECSXP, 5));
    SET_VECTOR_ELT(ans, 0, i);
    SET_VECTOR_ELT(ans, 1, d);
    SET_VECTOR_ELT(ans, 2, l);
    SET_VECTOR_ELT(ans, 3, s);
    SET_VECTOR_ELT(ans, 4, c);
    UNPROTECT(6);
    return ans;
}

// The scalar coercions: asInteger, asLogical, asReal, asChar.
SEXP capi_as(SEXP x) {
    SEXP ans = PROTECT(allocVector(VECSXP, 4));
    SET_VECTOR_ELT(ans, 0, ScalarInteger(asInteger(x)));
    SET_VECTOR_ELT(ans, 1, ScalarLogical(asLogical(x)));
    SET_VECTOR_ELT(ans, 2, ScalarReal(asReal(x)));
    SET_VECTOR_ELT(ans, 3, ScalarString(asChar(x)));
    UNPROTECT(1);
    return ans;
}

// asBool errors on NA, so it gets its own (always-TRUE) inputs.
SEXP capi_asbool(SEXP x) {
    return ScalarLogical(asBool(x));
}

// type2char/type2str/str2type round trips. type2str returns a CHARSXP.
// R: main/util.c:type2char
SEXP capi_type2(void) {
    SEXP t = PROTECT(type2str(INTSXP));
    SEXP back = PROTECT(ScalarInteger(str2type(CHAR(t))));
    SEXP ans = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 0, mkString(type2char(REALSXP)));
    SET_VECTOR_ELT(ans, 1, ScalarString(t));
    SET_VECTOR_ELT(ans, 2, back);
    UNPROTECT(3);
    return ans;
}

// The is* predicates, one row per value in x, one column per predicate.
SEXP capi_predicates(SEXP x) {
    SEXP ans = PROTECT(allocVector(LGLSXP, 20 * LENGTH(x)));
    for (int i = 0; i < LENGTH(x); i++) {
        SEXP v = VECTOR_ELT(x, i);
        LOGICAL(ans)[20 * i + 0] = isNull(v);
        LOGICAL(ans)[20 * i + 1] = isInteger(v);
        LOGICAL(ans)[20 * i + 2] = isReal(v);
        LOGICAL(ans)[20 * i + 3] = isLogical(v);
        LOGICAL(ans)[20 * i + 4] = isString(v);
        LOGICAL(ans)[20 * i + 5] = isVector(v);
        LOGICAL(ans)[20 * i + 6] = isVectorAtomic(v);
        LOGICAL(ans)[20 * i + 7] = isVectorList(v);
        LOGICAL(ans)[20 * i + 8] = isNewList(v);
        LOGICAL(ans)[20 * i + 9] = isList(v);
        LOGICAL(ans)[20 * i + 10] = isSymbol(v);
        LOGICAL(ans)[20 * i + 11] = isEnvironment(v);
        LOGICAL(ans)[20 * i + 12] = isFunction(v);
        LOGICAL(ans)[20 * i + 13] = isNumeric(v);
        LOGICAL(ans)[20 * i + 14] = isFactor(v);
        LOGICAL(ans)[20 * i + 15] = isMatrix(v);
        LOGICAL(ans)[20 * i + 16] = isArray(v);
        LOGICAL(ans)[20 * i + 17] = isDataFrame(v);
        LOGICAL(ans)[20 * i + 18] = isObject(v);
        LOGICAL(ans)[20 * i + 19] = isS4(v);
    }
    UNPROTECT(1);
    return ans;
}

// nrows/ncols and asCharacterFactor.
SEXP capi_dims(SEXP x) {
    SEXP f = PROTECT(allocVector(INTSXP, 3));
    for (int i = 0; i < 3; i++) INTEGER(f)[i] = i + 1;
    SEXP lv = PROTECT(allocVector(STRSXP, 3));
    for (int i = 0; i < 3; i++) {
        char buf[2] = {(char)('a' + i), 0};
        SET_STRING_ELT(lv, i, mkChar(buf));
    }
    setAttrib(f, R_LevelsSymbol, lv);
    setAttrib(f, R_ClassSymbol, mkString("factor"));
    SEXP ans = PROTECT(allocVector(VECSXP, 4));
    SET_VECTOR_ELT(ans, 0, ScalarInteger(nrows(x)));
    SET_VECTOR_ELT(ans, 1, ScalarInteger(ncols(x)));
    SET_VECTOR_ELT(ans, 2, f);
    SET_VECTOR_ELT(ans, 3, asCharacterFactor(f));
    UNPROTECT(3);
    return ans;
}
