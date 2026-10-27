// ALTREP: a custom compact integer sequence and a string wrapper class,
// modelled on R's own example classes.
// R: main/altclasses.c (compact_intseq, wrapper_* classes)
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Altrep.h>

// ALTREP accessors exported by libR but declared only in R's Defn.h.
R_xlen_t ALTREP_LENGTH(SEXP x);
void *ALTVEC_DATAPTR(SEXP x);
const void *ALTVEC_DATAPTR_RO(SEXP x);
SEXP ALTSTRING_ELT(SEXP x, R_xlen_t i);

// ---- capi_seq: the integers start, start+step, ..., length n -----------

static R_altrep_class_t capi_seq_class;

// data1 is a REALSXP triple (start, step, n).
static R_xlen_t capi_seq_Length(SEXP x) {
    return (R_xlen_t)REAL(R_altrep_data1(x))[2];
}

static int capi_seq_Elt(SEXP x, R_xlen_t i) {
    double *info = REAL(R_altrep_data1(x));
    return (int)(info[0] + info[1] * i);
}

static R_xlen_t capi_seq_Get_region(SEXP x, R_xlen_t i, R_xlen_t n, int *buf) {
    double *info = REAL(R_altrep_data1(x));
    R_xlen_t len = (R_xlen_t)info[2];
    if (i > len) return 0;
    if (i + n > len) n = len - i;
    for (R_xlen_t k = 0; k < n; k++)
        buf[k] = (int)(info[0] + info[1] * (i + k));
    return n;
}

// Materialize into a plain integer vector when R asks for the data.
static void *capi_seq_Dataptr(SEXP x, Rboolean writable) {
    R_xlen_t n = ALTREP_LENGTH(x);
    SEXP v = allocVector(INTSXP, n);
    for (R_xlen_t i = 0; i < n; i++)
        INTEGER(v)[i] = capi_seq_Elt(x, i);
    R_set_altrep_data2(x, v);  // keep the standard object reachable
    return INTEGER(v);
}

// No data pointer until asked for (the point of ALTREP).
static const void *capi_seq_Dataptr_or_null(SEXP x) {
    return NULL;
}

static SEXP capi_seq_Duplicate(SEXP x, Rboolean deep) {
    return R_new_altrep(capi_seq_class, R_altrep_data1(x), R_NilValue);
}

static Rboolean capi_seq_Inspect(SEXP x, int pre, int deep, int pvec,
                                 void (*inspect_subtree)(SEXP, int, int, int)) {
    Rprintf("capi_seq, length %lld\n", (long long)ALTREP_LENGTH(x));
    return FALSE;
}

static int capi_seq_Sorted(SEXP x) { return SORTED_INCR; }
static int capi_seq_no_NA(SEXP x) { return 1; }

static SEXP capi_seq_Sum(SEXP x, Rboolean narm) {
    R_xlen_t n = ALTREP_LENGTH(x), s = 0;
    for (R_xlen_t i = 0; i < n; i++) s += capi_seq_Elt(x, i);
    return ScalarInteger((int)s);
}

// ---- capi_strs: an ALTSTRING wrapping a STRSXP -------------------------

static R_altrep_class_t capi_strs_class;

static SEXP capi_strs_data(SEXP x) { return R_altrep_data1(x); }

static R_xlen_t capi_strs_Length(SEXP x) {
    return LENGTH(capi_strs_data(x));
}

static SEXP capi_strs_Elt(SEXP x, R_xlen_t i) {
    return STRING_ELT(capi_strs_data(x), i);
}

static void capi_strs_Set_elt(SEXP x, R_xlen_t i, SEXP v) {
    SET_STRING_ELT(capi_strs_data(x), i, v);
}

static SEXP capi_strs_Duplicate(SEXP x, Rboolean deep) {
    return R_new_altrep(capi_strs_class, duplicate(capi_strs_data(x)), R_NilValue);
}

// ---- construction and the API-side queries ----------------------------

void capi_init_altrep(void) {
    capi_seq_class = R_make_altinteger_class("capi_seq", "capi", NULL);
    R_set_altrep_Length_method(capi_seq_class, capi_seq_Length);
    R_set_altrep_Duplicate_method(capi_seq_class, capi_seq_Duplicate);
    R_set_altrep_Inspect_method(capi_seq_class, capi_seq_Inspect);
    R_set_altvec_Dataptr_method(capi_seq_class, capi_seq_Dataptr);
    R_set_altvec_Dataptr_or_null_method(capi_seq_class, capi_seq_Dataptr_or_null);
    R_set_altinteger_Elt_method(capi_seq_class, capi_seq_Elt);
    R_set_altinteger_Get_region_method(capi_seq_class, capi_seq_Get_region);
    R_set_altinteger_Is_sorted_method(capi_seq_class, capi_seq_Sorted);
    R_set_altinteger_No_NA_method(capi_seq_class, capi_seq_no_NA);
    R_set_altinteger_Sum_method(capi_seq_class, capi_seq_Sum);

    capi_strs_class = R_make_altstring_class("capi_strs", "capi", NULL);
    R_set_altrep_Length_method(capi_strs_class, capi_strs_Length);
    R_set_altrep_Duplicate_method(capi_strs_class, capi_strs_Duplicate);
    R_set_altstring_Elt_method(capi_strs_class, capi_strs_Elt);
    R_set_altstring_Set_elt_method(capi_strs_class, capi_strs_Set_elt);
}

SEXP capi_altrep_seq(void) {
    if (R_SEXP(capi_seq_class) == NULL) capi_init_altrep();
    SEXP info = PROTECT(allocVector(REALSXP, 3));
    REAL(info)[0] = 10;  // start
    REAL(info)[1] = 2;   // step
    REAL(info)[2] = 5;   // n
    SEXP ans = R_new_altrep(capi_seq_class, info, R_NilValue);
    UNPROTECT(1);
    return ans;
}

SEXP capi_altrep_strs(SEXP s) {
    if (R_SEXP(capi_strs_class) == NULL) capi_init_altrep();
    return R_new_altrep(capi_strs_class, s, R_NilValue);
}

// The queries: ALTREP, ALTREP_LENGTH, ALTREP_CLASS, R_altrep_inherits,
// ALTINTEGER_ELT, ALTSTRING_ELT, ALTVEC_DATAPTR(_RO).
SEXP capi_altrep_queries(SEXP s) {
    SEXP sq = PROTECT(capi_altrep_seq());
    SEXP st = PROTECT(capi_altrep_strs(s));
    SEXP ans = PROTECT(allocVector(VECSXP, 10));
    SET_VECTOR_ELT(ans, 0, ScalarLogical(ALTREP(sq)));
    SET_VECTOR_ELT(ans, 1, ScalarInteger(ALTREP_LENGTH(sq)));
    SET_VECTOR_ELT(ans, 2, mkString(type2char(TYPEOF(ALTREP_CLASS(sq)))));
    SET_VECTOR_ELT(ans, 3, ScalarLogical(R_altrep_inherits(sq, capi_seq_class)));
    SET_VECTOR_ELT(ans, 4, ScalarInteger(INTEGER_ELT(sq, 2)));
    SET_VECTOR_ELT(ans, 5, ScalarString(ALTSTRING_ELT(st, 1)));
    SET_VECTOR_ELT(ans, 6, st);
    SET_VECTOR_ELT(ans, 7, ScalarLogical(ALTVEC_DATAPTR_RO(sq) == NULL));
    SET_VECTOR_ELT(ans, 8, ScalarInteger(((int *)ALTVEC_DATAPTR(sq))[4]));
    // materialization stored the standard vector in data2; read it back
    SET_VECTOR_ELT(ans, 9, R_altrep_data2(sq));
    UNPROTECT(3);
    return ans;
}
// placeholder
