// Pairlists and calls: cons cells and the langN/listN constructors.
// R: main/builtin.c:listfunctions, main/cons.c
#include <R.h>
#include <Rinternals.h>

// cons/lcons/allocList and the accessors.
SEXP capi_cons(void) {
    SEXP l = PROTECT(cons(ScalarInteger(1), R_NilValue));
    SET_TAG(l, install("a"));
    SEXP tail = l;
    for (int i = 2; i <= 3; i++) {
        SEXP cell = PROTECT(cons(ScalarInteger(i), R_NilValue));
        SETCDR(tail, cell);
        tail = cell;
        UNPROTECT(1);
    }
    int nth = asInteger(CAR(nthcdr(l, 2)));
    SEXP ans = PROTECT(allocVector(VECSXP, 8));
    SET_VECTOR_ELT(ans, 0, l);
    SET_VECTOR_ELT(ans, 1, ScalarInteger(asInteger(CAR(l))));
    SET_VECTOR_ELT(ans, 2, ScalarInteger(asInteger(CADR(l))));
    SET_VECTOR_ELT(ans, 3, ScalarInteger(asInteger(CADDR(l))));
    SET_VECTOR_ELT(ans, 4, ScalarInteger(asInteger(CAR(CDDR(l)))));
    SET_VECTOR_ELT(ans, 5, ScalarInteger(nth));
    SET_VECTOR_ELT(ans, 6, mkString(CHAR(PRINTNAME(TAG(l)))));
    SET_VECTOR_ELT(ans, 7, ScalarInteger(length(allocList(4))));
    UNPROTECT(2);
    return ans;
}

// Mutation: SETCAR/SETCDR/SET_TAG/SETCADR on a copy.
SEXP capi_cons_mutate(SEXP lang) {
    SEXP l = PROTECT(duplicate(lang));
    SETCAR(l, install("*"));
    SETCADR(l, ScalarInteger(6));
    SET_TAG(l, R_NilValue);  // calls have no tags
    SEXP tail = CDR(l);
    SETCAR(tail, ScalarInteger(7));
    UNPROTECT(1);
    return l;
}

// langN build language objects; listN build (unnamed) lists.
SEXP capi_langs(void) {
    SEXP l1 = PROTECT(lang1(install("sqrt")));
    SEXP l2 = PROTECT(lang2(install("sqrt"), ScalarInteger(9)));
    SEXP l3 = PROTECT(lang3(install("+"), ScalarInteger(1), ScalarInteger(2)));
    SEXP l4 = PROTECT(lang4(install("+"), ScalarInteger(1), ScalarInteger(2),
                            ScalarInteger(3)));
    SEXP s2 = PROTECT(list2(ScalarInteger(1), ScalarReal(2.5)));
    SEXP lc = PROTECT(lcons(install("+"), list2(ScalarInteger(20), ScalarInteger(22))));
    SEXP ans = PROTECT(allocVector(VECSXP, 6));
    SET_VECTOR_ELT(ans, 0, l1);
    SET_VECTOR_ELT(ans, 1, l2);
    SET_VECTOR_ELT(ans, 2, l3);
    SET_VECTOR_ELT(ans, 3, l4);
    SET_VECTOR_ELT(ans, 4, s2);
    SET_VECTOR_ELT(ans, 5, lc);  // lcons: the call builder
    UNPROTECT(7);
    return ans;
}

// VectorToPairList / PairToVectorList / lengthgets / xlengthgets.
SEXP capi_convert(SEXP x) {
    SEXP pl = PROTECT(VectorToPairList(x));
    SEXP back = PROTECT(PairToVectorList(pl));
    SEXP grown = PROTECT(lengthgets(x, 5));
    SEXP xgrown = PROTECT(xlengthgets(back, 2));
    SEXP ans = PROTECT(allocVector(VECSXP, 4));
    SET_VECTOR_ELT(ans, 0, pl);
    SET_VECTOR_ELT(ans, 1, back);
    SET_VECTOR_ELT(ans, 2, grown);
    SET_VECTOR_ELT(ans, 3, xgrown);
    UNPROTECT(5);
    return ans;
}

// Alias visibility: GNU R's SETCAR/SETCDR/SET_TAG mutate the caller's cells
// in place (main/memory.c); they do not duplicate. Every binding of the
// object sees the new cell, as the R helpers expect.
SEXP capi_setcar_alias(SEXP x) {
    SETCAR(x, ScalarInteger(99));
    return x;
}

SEXP capi_setcar2_alias(SEXP x) {
    SETCAR(CDR(x), ScalarInteger(98));
    return x;
}

SEXP capi_setcdr_alias(SEXP x, SEXP tail) {
    SETCDR(x, tail);
    return x;
}

SEXP capi_settag_alias(SEXP x, SEXP tag) {
    SET_TAG(x, tag);
    return x;
}

SEXP capi_setfun_alias(SEXP x) {
    SETCAR(x, install("*"));
    SETCAR(CDR(x), ScalarInteger(6));
    return x;
}
