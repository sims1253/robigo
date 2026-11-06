// A package ALTREP character vector that materializes on first use, ported
// from vctrs (src/altrep-lazy-character.c), with counters on the methods
// that read it: Length, Elt, Dataptr, Duplicate. dplyr's test-generics
// (#6525) checks that its row names stay lazy through attribute work; the
// helpers below are dplyr's ffi_test_dplyr_attributes/_set_attributes
// (src/reconstruct.cpp) and a shallow reconstruct (#468).
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Altrep.h>
#include <R_ext/Rdynload.h>

static R_altrep_class_t lazy_chr_class;
static int n_length = 0, n_elt = 0, n_dataptr = 0, n_duplicate = 0;

static SEXP lazy_materialize(SEXP vec) {
    SEXP out = R_altrep_data2(vec);
    if (out != R_NilValue) return out;
    SEXP call = PROTECT(Rf_lang1(R_altrep_data1(vec)));
    out = Rf_eval(call, R_GlobalEnv);
    if (TYPEOF(out) != STRSXP) Rf_error("`fn` must evaluate to a character vector.");
    R_set_altrep_data2(vec, out);
    UNPROTECT(1);
    return out;
}

static void *lazy_Dataptr(SEXP vec, Rboolean writeable) {
    n_dataptr++;
    if (writeable) Rf_error("Can't get writeable `DATAPTR()` to `<lazy_chr>`");
    return (void *) STRING_PTR_RO(lazy_materialize(vec));
}

static const void *lazy_Dataptr_or_null(SEXP vec) {
    SEXP out = R_altrep_data2(vec);
    return out == R_NilValue ? NULL : (const void *) STRING_PTR_RO(out);
}

static R_xlen_t lazy_Length(SEXP vec) {
    n_length++;
    return XLENGTH(lazy_materialize(vec));
}

static SEXP lazy_Elt(SEXP vec, R_xlen_t i) {
    n_elt++;
    return STRING_ELT(lazy_materialize(vec), i);
}

static void lazy_Set_elt(SEXP vec, R_xlen_t i, SEXP value) {
    SET_STRING_ELT(lazy_materialize(vec), i, value);
}

static SEXP lazy_Duplicate(SEXP vec, Rboolean deep) {
    n_duplicate++;
    return NULL;  // altrep.c's default: a standard copy
}

static Rboolean lazy_Inspect(SEXP x, int pre, int deep, int pvec,
                             void (*inspect_subtree)(SEXP, int, int, int)) {
    Rprintf(" lazy_chr (materialized=%s)\n", R_altrep_data2(x) != R_NilValue ? "T" : "F");
    return TRUE;
}

SEXP lazy_new(SEXP fn) {
    SEXP out = R_new_altrep(lazy_chr_class, fn, R_NilValue);
    MARK_NOT_MUTABLE(out);
    return out;
}

SEXP lazy_is_materialized(SEXP x) {
    if (!ALTREP(x) || !R_altrep_inherits(x, lazy_chr_class)) Rf_error("not a lazy_chr");
    return Rf_ScalarLogical(R_altrep_data2(x) != R_NilValue);
}

// the method calls since the last call, then reset
SEXP lazy_counts(void) {
    SEXP out = PROTECT(Rf_allocVector(INTSXP, 4));
    INTEGER(out)[0] = n_length;
    INTEGER(out)[1] = n_elt;
    INTEGER(out)[2] = n_dataptr;
    INTEGER(out)[3] = n_duplicate;
    SEXP nms = PROTECT(Rf_allocVector(STRSXP, 4));
    SET_STRING_ELT(nms, 0, Rf_mkChar("Length"));
    SET_STRING_ELT(nms, 1, Rf_mkChar("Elt"));
    SET_STRING_ELT(nms, 2, Rf_mkChar("Dataptr"));
    SET_STRING_ELT(nms, 3, Rf_mkChar("Duplicate"));
    Rf_setAttrib(out, R_NamesSymbol, nms);
    n_length = n_elt = n_dataptr = n_duplicate = 0;
    UNPROTECT(2);
    return out;
}

// dplyr's ffi_test_dplyr_attributes
SEXP lazy_attributes(SEXP x) {
    return ATTRIB(x);
}

// dplyr's ffi_test_dplyr_set_attributes
SEXP lazy_set_attributes(SEXP x, SEXP attributes) {
    if (TYPEOF(attributes) != LISTSXP) Rf_error("`attributes` must be a pairlist.");
    x = PROTECT(Rf_shallow_duplicate(x));
    SET_ATTRIB(x, attributes);
    UNPROTECT(1);
    return x;
}

// dplyr_reconstruct's shape: a shallow copy of `data` (its attributes
// shared) with `template`'s class
SEXP lazy_reconstruct(SEXP data, SEXP template_) {
    SEXP out = PROTECT(Rf_shallow_duplicate(data));
    Rf_setAttrib(out, R_ClassSymbol, Rf_getAttrib(template_, R_ClassSymbol));
    UNPROTECT(1);
    return out;
}

static const R_CallMethodDef calls[] = {
    {"lazy_new", (DL_FUNC) &lazy_new, 1},
    {"lazy_is_materialized", (DL_FUNC) &lazy_is_materialized, 1},
    {"lazy_counts", (DL_FUNC) &lazy_counts, 0},
    {"lazy_attributes", (DL_FUNC) &lazy_attributes, 1},
    {"lazy_set_attributes", (DL_FUNC) &lazy_set_attributes, 2},
    {"lazy_reconstruct", (DL_FUNC) &lazy_reconstruct, 2},
    {NULL, NULL, 0}
};

void R_init_altrep_lazy_string(DllInfo *dll) {
    R_registerRoutines(dll, NULL, calls, NULL, NULL);
    lazy_chr_class = R_make_altstring_class("lazy_chr", "altrep_lazy_string", dll);
    R_set_altvec_Dataptr_method(lazy_chr_class, lazy_Dataptr);
    R_set_altvec_Dataptr_or_null_method(lazy_chr_class, lazy_Dataptr_or_null);
    R_set_altrep_Length_method(lazy_chr_class, lazy_Length);
    R_set_altrep_Duplicate_method(lazy_chr_class, lazy_Duplicate);
    R_set_altrep_Inspect_method(lazy_chr_class, lazy_Inspect);
    R_set_altstring_Elt_method(lazy_chr_class, lazy_Elt);
    R_set_altstring_Set_elt_method(lazy_chr_class, lazy_Set_elt);
}
