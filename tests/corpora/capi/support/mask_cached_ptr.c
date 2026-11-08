#include <R.h>
#include <Rinternals.h>

/* dplyr's data-mask loop in miniature: the group id is an integer
   binding C finds once with findVarInFrame, caches the raw pointer
   of, and rewrites between evaluations of an expression that reads
   it through an active binding. The second write goes through the
   cached pointer alone: no DATAPTR call reports it. */
SEXP capi_mask_loop(SEXP info, SEXP mask) {
    SEXP idx = PROTECT(findVarInFrame(info, install("pkg:::current_group_id")));
    if (TYPEOF(idx) != INTSXP || XLENGTH(idx) < 1)
        error("group id is not a non-empty integer");
    int* p = INTEGER(idx);
    SEXP out = PROTECT(allocVector(VECSXP, 2));
    *p = 1;
    SEXP v1 = PROTECT(eval(install("colx"), mask));
    SET_VECTOR_ELT(out, 0, v1);
    *p = 2;
    SEXP v2 = PROTECT(eval(install("colx"), mask));
    SET_VECTOR_ELT(out, 1, v2);
    UNPROTECT(4);
    return out;
}

/* A fresh 0L from C, as Rf_duplicate(0L) hands to R (dplyr binds
   its group id that way: not a shared scalar literal). */
SEXP capi_fresh_zero(void) {
    SEXP z = PROTECT(ScalarInteger(0));
    SEXP d = Rf_duplicate(z);
    UNPROTECT(1);
    return d;
}

/* A splice box in rlang 1.1's shape: its identity is the attribute
   pairlist itself, one preserved node every box shares. */
/* The boxes' shared attribute pairlist, preserved once for the
   session (R_PreserveObject): intentionally not a leak. */
static SEXP box_attrib = NULL;
SEXP capi_box_init(void) {
    if (!box_attrib) {
        SEXP cls = PROTECT(allocVector(STRSXP, 1));
        SET_STRING_ELT(cls, 0, mkChar("my_box"));
        box_attrib = allocList(1);
        SETCAR(box_attrib, cls);
        SET_TAG(box_attrib, R_ClassSymbol);
        R_PreserveObject(box_attrib);
        UNPROTECT(1);
    }
    return R_NilValue;
}
SEXP capi_box_make(SEXP x) {
    SEXP out = PROTECT(allocVector(VECSXP, 1));
    SET_VECTOR_ELT(out, 0, x);
    SET_ATTRIB(out, box_attrib);
    SET_OBJECT(out, 1);
    UNPROTECT(1);
    return out;
}
SEXP capi_box_is(SEXP x) { return ScalarLogical(ATTRIB(x) == box_attrib); }

/* PairToVectorList through Rf_coerceVector moves the elements
   themselves: two boxes keep the one shared attribute pairlist. */
SEXP capi_box_pair(void) {
    SEXP dots = PROTECT(allocList(2));
    SEXP x1 = PROTECT(ScalarInteger(1));
    SEXP a = PROTECT(capi_box_make(x1));
    SEXP x2 = PROTECT(ScalarInteger(2));
    SEXP b = PROTECT(capi_box_make(x2));
    SETCAR(dots, a);
    SETCAR(CDR(dots), b);
    SEXP v = Rf_coerceVector(dots, VECSXP);
    UNPROTECT(5);
    return v;
}
