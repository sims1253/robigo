// Allocation, PROTECT and the scalar constructors.
// R: main/memory.c:allocVector, main/protect.c
#include <R.h>
#include <Rinternals.h>

// Exported by libR but not declared in any public header; packages that
// use them declare them themselves.
extern SEXP R_TrueValue, R_FalseValue, R_LogicalNAValue;
extern SEXP *R_PPStack;
extern int R_PPStackTop, R_PPStackSize;

// allocVector of every element type; length/xlength/LENGTH/XLENGTH of each.
SEXP capi_alloc_types(void) {
    SEXP ans = PROTECT(allocVector(VECSXP, 8));
    SET_VECTOR_ELT(ans, 0, allocVector(LGLSXP, 3));
    SET_VECTOR_ELT(ans, 1, allocVector(INTSXP, 3));
    SET_VECTOR_ELT(ans, 2, allocVector(REALSXP, 3));
    SET_VECTOR_ELT(ans, 3, allocVector(CPLXSXP, 3));
    SET_VECTOR_ELT(ans, 4, allocVector(STRSXP, 3));
    SET_VECTOR_ELT(ans, 5, allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 6, allocVector(EXPRSXP, 3));
    SET_VECTOR_ELT(ans, 7, allocVector(RAWSXP, 3));
    UNPROTECT(1);
    return ans;
}

// Rf_length on vectors of several types and on R_NilValue.
SEXP capi_lengths(SEXP x) {
    int extra = isNull(x) ? 0 : LENGTH(x) + XLENGTH(x);
    return ScalarInteger(length(x) + xlength(x) + extra);
}

// The scalar constructors and R's constant data values.
SEXP capi_scalars(void) {
    SEXP ans = PROTECT(allocVector(VECSXP, 13));
    SET_VECTOR_ELT(ans, 0, ScalarInteger(NA_INTEGER));
    SET_VECTOR_ELT(ans, 1, ScalarInteger(42));
    SET_VECTOR_ELT(ans, 2, ScalarReal(R_NaReal));
    SET_VECTOR_ELT(ans, 3, ScalarReal(R_PosInf));
    SET_VECTOR_ELT(ans, 4, ScalarReal(R_NaN));
    SET_VECTOR_ELT(ans, 5, ScalarReal(R_NegInf));
    SET_VECTOR_ELT(ans, 6, ScalarLogical(NA_LOGICAL));
    SET_VECTOR_ELT(ans, 7, ScalarLogical(TRUE));
    SET_VECTOR_ELT(ans, 8, ScalarLogical(FALSE));
    SET_VECTOR_ELT(ans, 9, R_TrueValue);
    SET_VECTOR_ELT(ans, 10, R_FalseValue);
    SET_VECTOR_ELT(ans, 11, R_LogicalNAValue);
    SET_VECTOR_ELT(ans, 12, ScalarRaw(65));
    UNPROTECT(1);
    return ans;
}

// PROTECT/UNPROTECT pairs; reprotecting a slot grown in a loop.
SEXP capi_protect(void) {
    PROTECT_INDEX ix;
    SEXP v = allocVector(REALSXP, 1);
    PROTECT_WITH_INDEX(v, &ix);
    REAL(v)[0] = 1.5;
    for (int i = 2; i <= 5; i++) {
        SEXP w = allocVector(REALSXP, 1);
        REAL(w)[0] = i;
        REPROTECT(w, ix);
        v = w;
    }
    double got = REAL(v)[0];
    UNPROTECT(1);
    return ScalarReal(got);
}

SEXP capi_unprotect_ptr(void) {
    SEXP a = PROTECT(allocVector(INTSXP, 1));
    SEXP b = PROTECT(allocVector(INTSXP, 1));
    INTEGER(a)[0] = 1;
    INTEGER(b)[0] = 2;
    UNPROTECT_PTR(a);  // takes the pointer, not a count
    UNPROTECT(1);
    return ScalarInteger(INTEGER(b)[0]);
}

SEXP capi_preserve(void) {
    SEXP v = allocVector(REALSXP, 1);
    R_PreserveObject(v);
    REAL(v)[0] = 3.25;
    R_ReleaseObject(v);
    return v;
}

// Matrices, raw sexp allocation and the memory checkers.
// R: main/memory.c:allocMatrix, R_chk_*
SEXP capi_alloc_more(void) {
    SEXP m = PROTECT(allocMatrix(INTSXP, 2, 3));
    for (int i = 0; i < 6; i++) INTEGER(m)[i] = i;
    SEXP v3 = PROTECT(allocVector3(REALSXP, 2, NULL));  // custom allocator: none
    REAL(v3)[0] = 1.5;
    REAL(v3)[1] = 0.0;
    SEXP cell = PROTECT(allocSExp(LISTSXP));  // a bare cons cell
    SETCAR(cell, ScalarInteger(5));
    SEXP s4 = PROTECT(allocS4Object());
    void *p = R_chk_calloc(4, 1);
    memcpy(p, "capi", 4);
    char dst[5];
    R_chk_memcpy(dst, p, 4);
    dst[4] = 0;
    R_chk_free(p);
    R_CheckStack2(64);  // C stack space check, returns
    R_CheckStack();
    // The protect stack: the top slot holds the newest protected object.
    SEXP ans = PROTECT(allocVector(VECSXP, 6));
    SET_VECTOR_ELT(ans, 0, m);
    SET_VECTOR_ELT(ans, 1, v3);
    SET_VECTOR_ELT(ans, 2, cell);
    SET_VECTOR_ELT(ans, 3, ScalarLogical(isS4(s4)));
    SET_VECTOR_ELT(ans, 4, ScalarLogical(R_PPStackTop < R_PPStackSize
                                         && R_PPStack[R_PPStackTop - 1] != NULL));
    SET_VECTOR_ELT(ans, 5, mkString(dst));
    UNPROTECT(6);
    return ans;
}
// R's (v)stack allocator: R_alloc/S_alloc/S_realloc zero what they add,
// R_chk_* wrap calloc/realloc/free and error on failure.
// R: main/memory.c:S_alloc, S_realloc, R_chk_calloc
SEXP capi_valloc(void) {
    void *vmax = vmaxget();
    double *a = (double *)R_alloc(4, sizeof(double));
    for (int i = 0; i < 4; i++) a[i] = i;
    double *b = (double *)S_realloc((char *)a, 8, 4, sizeof(double));
    int *c = (int *)S_alloc(3, sizeof(int));
    void *p = R_chk_calloc(4, sizeof(double));
    p = R_chk_realloc(p, 8 * sizeof(double));
    R_chk_free(p);
    vmaxset(vmax);
    return ScalarReal(b[0] + b[3] + b[7] + c[0] + c[2]);
}
