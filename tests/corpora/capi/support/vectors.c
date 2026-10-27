// Vector data access: pointer families, *_ELT accessors, NA/Inf tests.
// R: main/memory.c:DATAPTR, main/sequence.c for the ALTREP accessors
#include <R.h>
#include <Rinternals.h>
#include <string.h>

// Write through INTEGER, read back through INTEGER_ELT/INTEGER_RO.
SEXP capi_int(SEXP x) {
    for (int i = 0; i < LENGTH(x); i++)
        INTEGER(x)[i] = INTEGER_ELT(x, i) * 2 + 1;
    SET_INTEGER_ELT(x, 0, 100L);
    const int *ro = INTEGER_RO(x);
    return ScalarInteger(ro[0] + ro[1]);
}

// REAL/REAL_ELT/REAL_RO.
SEXP capi_real(SEXP x) {
    double s = 0;
    for (int i = 0; i < LENGTH(x); i++) s += REAL_ELT(x, i);
    const double *ro = REAL_RO(x);
    return ScalarReal(s + ro[0]);
}

// LOGICAL_ELT/SET_LOGICAL_ELT and RAW.
SEXP capi_lgl_raw(void) {
    SEXP l = PROTECT(allocVector(LGLSXP, 3));
    SET_LOGICAL_ELT(l, 0, 1);
    SET_LOGICAL_ELT(l, 1, LOGICAL_ELT(l, 0));
    SET_LOGICAL_ELT(l, 2, 0);
    SEXP r = PROTECT(allocVector(RAWSXP, 4));
    for (int i = 0; i < 3; i++) RAW(r)[i] = (Rbyte)('A' + i);
    RAW(r)[3] = 0;  // CHARSXPs are NUL-terminated; our buffer must be too
    SEXP ans = PROTECT(allocVector(STRSXP, 2));
    SET_STRING_ELT(ans, 0, mkChar((const char *)RAW(r)));
    SET_STRING_ELT(ans, 1, LOGICAL_ELT(l, 2) == 0 ? mkChar("zero") : R_NaString);
    UNPROTECT(3);
    return ans;
}

// COMPLEX access, both the pointer and COMPLEX_ELT.
SEXP capi_complex(SEXP x) {
    Rcomplex z = COMPLEX_ELT(x, 0);
    Rcomplex w = COMPLEX(x)[1];
    Rcomplex s;
    s.r = z.r + w.r;
    s.i = z.i + w.i;
    SEXP ans = PROTECT(allocVector(CPLXSXP, 1));
    SET_COMPLEX_ELT(ans, 0, s);
    UNPROTECT(1);
    return ans;
}

// R_IsNA, R_IsNaN, R_finite over c(NA, NaN, Inf, -Inf, 0).
// R: main/arith.c:R_IsNA
SEXP capi_na(SEXP x) {
    SEXP ans = PROTECT(allocVector(LGLSXP, LENGTH(x)));
    for (int i = 0; i < LENGTH(x); i++)
        LOGICAL(ans)[i] = R_IsNA(REAL_ELT(x, i)) + 2 * R_IsNaN(REAL_ELT(x, i))
                           + 4 * R_finite(REAL_ELT(x, i));
    UNPROTECT(1);
    return ans;
}

// List/vector/string element access, R_NaString, R_BlankString,
// the _PTR_RO pointers and DATAPTR_RO.
SEXP capi_lists(SEXP x, SEXP s) {
    SEXP y = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(y, 0, VECTOR_ELT(x, 0));
    SET_VECTOR_ELT(y, 1, VECTOR_PTR_RO(x)[1]);
    double sum = 0;
    const double *d = (const double *)DATAPTR_RO(VECTOR_ELT(x, 0));
    for (int i = 0; i < LENGTH(VECTOR_ELT(x, 0)); i++) sum += d[i];
    SEXP t = PROTECT(allocVector(STRSXP, 3));
    SET_STRING_ELT(t, 0, STRING_ELT(s, 1));
    SET_STRING_ELT(t, 1, R_BlankString);
    SET_STRING_ELT(t, 2, R_NaString);
    const SEXP *sp = STRING_PTR_RO(t);
    SEXP ans = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(ans, 0, ScalarReal(sum));
    SET_VECTOR_ELT(ans, 1, ScalarString(sp[0]));
    UNPROTECT(3);
    return ans;
}

// The _RO/_OR_NULL accessors for every type. ialt is an altrep integer
// sequence (1:6), rvec a plain real vector.
SEXP capi_accessors(SEXP cx, SEXP lx, SEXP rx, SEXP ialt, SEXP rvec) {
    SEXP ans = PROTECT(allocVector(LGLSXP, 9));
    LOGICAL(ans)[0] = COMPLEX_RO(cx)[0].r == COMPLEX_ELT(cx, 0).r;
    LOGICAL(ans)[1] = LOGICAL_RO(lx)[0] == LOGICAL_ELT(lx, 0);
    LOGICAL(ans)[2] = RAW_RO(rx)[0] == RAW(rx)[0];
    LOGICAL(ans)[3] = INTEGER_OR_NULL(ialt) == NULL;  // compact seq: no data yet
    LOGICAL(ans)[4] = REAL_OR_NULL(rvec) != NULL;     // standard vector: data
    LOGICAL(ans)[5] = IS_LONG_VEC(cx);
    LOGICAL(ans)[6] = LENGTH_EX(cx, "vectors.c", 1) == LENGTH(cx);
    LOGICAL(ans)[7] = REFCNT(cx) == REFCNT(cx);
    LOGICAL(ans)[8] = IS_LONG_VEC(allocVector(REALSXP, 3));
    UNPROTECT(1);
    return ans;
}

// Element setters and pairlist accessors beyond the common ones.
SEXP capi_setters(SEXP x) {
    SET_REAL_ELT(x, 1, 9.5);
    SEXP l = PROTECT(lang4(install("f"), ScalarInteger(1), ScalarInteger(2),
                           ScalarInteger(3)));
    SETCADDR(l, ScalarInteger(20));
    SETCADDDR(l, ScalarInteger(30));
    SEXP a4 = PROTECT(ScalarInteger(asInteger(CAD4R(l))));
    SEXP a3 = PROTECT(ScalarInteger(asInteger(CADDDR(l))));
    SEXP c4 = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(c4, 0, l);
    SET_VECTOR_ELT(c4, 1, a4);
    SET_VECTOR_ELT(c4, 2, a3);
    UNPROTECT(4);
    return c4;
}

// ScalarComplex, duplication from C, identical from C, sorting.
// R: main/unique.c:Rf_duplicated, main/identical:R_compute_identical
SEXP capi_identical(void) {
    SEXP c = PROTECT(ScalarComplex((Rcomplex){.r = 1.5, .i = 2.5}));
    SEXP a = PROTECT(allocVector(REALSXP, 5));
    for (int i = 0; i < 5; i++) REAL(a)[i] = 5 - i;
    SEXP dup = PROTECT(duplicated(a, FALSE));
    R_xlen_t any1 = any_duplicated(a, FALSE);
    SEXP na_lgl = PROTECT(ScalarLogical(NA_LOGICAL));
    R_xlen_t any2 = any_duplicated3(a, na_lgl, FALSE);
    double b[5];
    for (int i = 0; i < 5; i++) b[i] = REAL(a)[i];
    R_rsort(b, 5);              // full sort
    double c2[5];
    for (int i = 0; i < 5; i++) c2[i] = REAL(a)[i];
    rPsort(c2, 5, 2);           // partial sort: element 2 in place
    SEXP ans = PROTECT(allocVector(VECSXP, 5));
    SET_VECTOR_ELT(ans, 0, c);
    SET_VECTOR_ELT(ans, 1, dup);
    SET_VECTOR_ELT(ans, 2, ScalarInteger((int)(any1 + any2)));
    SET_VECTOR_ELT(ans, 3, R_compute_identical(a, a, 0) ? a : R_NilValue);
    SEXP s = PROTECT(allocVector(REALSXP, 5));
    for (int i = 0; i < 5; i++) REAL(s)[i] = b[i];
    SET_VECTOR_ELT(ans, 4, s);
    UNPROTECT(6);
    return ans;
}

// Sharing flags: a fresh vector has no references; MARK_NOT_MUTABLE and
// MAYBE_SHARED/NO_REFERENCES report the state.
SEXP capi_sharing(void) {
    SEXP x = PROTECT(allocVector(REALSXP, 2));
    int fresh = NO_REFERENCES(x);
    MARK_NOT_MUTABLE(x);
    MAYBE_SHARED(x);  // read-only query, state not printed (GC-dependent)
    SEXP y = PROTECT(allocVector(STRSXP, 4));
    SET_GROWABLE_BIT(y);
    SETLENGTH(y, 2);
    SET_TRUELENGTH(y, 4);
    UNPROTECT(2);
    return ScalarInteger(fresh + 10 * LENGTH(y) + 100 * TRUELENGTH(y));
}
