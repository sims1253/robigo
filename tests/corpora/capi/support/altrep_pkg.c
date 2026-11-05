// ALTREP classes defined by a package, read through their methods where
// GNU R reads them (#422), modelled on cli's progress_along and
// cli_timer_t (cli 3.6.6, src/progress-altrep.c).
//
// count_seq: an integer vector whose data1 is a plain INTSXP 1..n. Elt
// counts its calls, optionally calls an R function with the 1-based index
// and raises an error at a chosen index; Get_region and Dataptr read data1
// without counting (as cli's do), so only the places that read through
// Elt move the counter. Sum and Max answer doubles, Min an integer;
// Is_sorted and No_NA answer hints; Inspect prints and recurses into data1;
// Serialized_state gives data1.
//
// live_lgl: a length-one logical whose value is a C global the class
// reads in Elt; setting the global does not touch the object (cli's timer
// flag is set by a signal handler).
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Altrep.h>
#include <R_ext/Rdynload.h>

static R_altrep_class_t count_seq_class;
static R_altrep_class_t live_lgl_class;
static int elt_calls = 0;
static int live_flag = 0;

// data2: list(callback or NULL, error index (0: none))
static R_xlen_t cs_Length(SEXP x) { return XLENGTH(R_altrep_data1(x)); }

static void *cs_Dataptr(SEXP x, Rboolean writeable) {
    return (void *) INTEGER(R_altrep_data1(x));
}

static const void *cs_Dataptr_or_null(SEXP x) {
    return (const void *) INTEGER(R_altrep_data1(x));
}

static int cs_Elt(SEXP x, R_xlen_t i) {
    elt_calls++;
    SEXP d2 = R_altrep_data2(x);
    SEXP cb = VECTOR_ELT(d2, 0);
    int err_at = INTEGER(VECTOR_ELT(d2, 1))[0];
    if (i + 1 == err_at)
        Rf_error("count_seq: no element %d", (int) (i + 1));
    if (cb != R_NilValue) {
        SEXP call = PROTECT(Rf_lang2(cb, PROTECT(ScalarInteger((int) (i + 1)))));
        Rf_eval(call, R_GlobalEnv);
        UNPROTECT(2);
    }
    return INTEGER(R_altrep_data1(x))[i];
}

static R_xlen_t cs_Get_region(SEXP x, R_xlen_t i, R_xlen_t n, int *buf) {
    return INTEGER_GET_REGION(R_altrep_data1(x), i, n, buf);
}

static SEXP cs_Sum(SEXP x, Rboolean narm) {
    double s = 0;
    R_xlen_t n = XLENGTH(R_altrep_data1(x));
    for (R_xlen_t i = 0; i < n; i++) s += INTEGER(R_altrep_data1(x))[i];
    return ScalarReal(s);
}

static SEXP cs_Max(SEXP x, Rboolean narm) {
    return ScalarReal((double) XLENGTH(R_altrep_data1(x)));
}

static SEXP cs_Min(SEXP x, Rboolean narm) {
    return ScalarInteger(1);
}

static int cs_Is_sorted(SEXP x) { return SORTED_INCR; }
static int cs_No_NA(SEXP x) { return 1; }

static Rboolean cs_Inspect(SEXP x, int pre, int deep, int pvec,
                           void (*inspect_subtree)(SEXP, int, int, int)) {
    Rprintf(" count_seq %s\n", type2char(TYPEOF(x)));
    inspect_subtree(R_altrep_data1(x), pre, deep, pvec);
    return TRUE;
}

static SEXP cs_Serialized_state(SEXP x) { return R_altrep_data1(x); }

static R_xlen_t ll_Length(SEXP x) { return 1; }
static void *ll_Dataptr(SEXP x, Rboolean writeable) { return (void *) &live_flag; }
static int ll_Elt(SEXP x, R_xlen_t i) { return live_flag; }

SEXP altrep_pkg_count_seq(SEXP n, SEXP cb, SEXP err_at) {
    int len = asInteger(n);
    SEXP d1 = PROTECT(allocVector(INTSXP, len));
    for (int i = 0; i < len; i++) INTEGER(d1)[i] = i + 1;
    SEXP d2 = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(d2, 0, cb);
    SET_VECTOR_ELT(d2, 1, ScalarInteger(asInteger(err_at)));
    SEXP x = R_new_altrep(count_seq_class, d1, d2);
    UNPROTECT(2);
    return x;
}

SEXP altrep_pkg_calls(void) {
    SEXP r = ScalarInteger(elt_calls);
    elt_calls = 0;
    return r;
}

SEXP altrep_pkg_live(void) {
    return R_new_altrep(live_lgl_class, R_NilValue, R_NilValue);
}

SEXP altrep_pkg_set_flag(SEXP v) {
    live_flag = asLogical(v);
    return R_NilValue;
}

static const R_CallMethodDef calls[] = {
    {"altrep_pkg_count_seq", (DL_FUNC) &altrep_pkg_count_seq, 3},
    {"altrep_pkg_calls", (DL_FUNC) &altrep_pkg_calls, 0},
    {"altrep_pkg_live", (DL_FUNC) &altrep_pkg_live, 0},
    {"altrep_pkg_set_flag", (DL_FUNC) &altrep_pkg_set_flag, 1},
    {NULL, NULL, 0}
};

void R_init_altrep_pkg(DllInfo *dll) {
    R_registerRoutines(dll, NULL, calls, NULL, NULL);
    count_seq_class = R_make_altinteger_class("count_seq", "altrep_pkg", dll);
    R_set_altrep_Length_method(count_seq_class, cs_Length);
    R_set_altrep_Inspect_method(count_seq_class, cs_Inspect);
    R_set_altrep_Serialized_state_method(count_seq_class, cs_Serialized_state);
    R_set_altvec_Dataptr_method(count_seq_class, cs_Dataptr);
    R_set_altvec_Dataptr_or_null_method(count_seq_class, cs_Dataptr_or_null);
    R_set_altinteger_Elt_method(count_seq_class, cs_Elt);
    R_set_altinteger_Get_region_method(count_seq_class, cs_Get_region);
    R_set_altinteger_Sum_method(count_seq_class, cs_Sum);
    R_set_altinteger_Max_method(count_seq_class, cs_Max);
    R_set_altinteger_Min_method(count_seq_class, cs_Min);
    R_set_altinteger_Is_sorted_method(count_seq_class, cs_Is_sorted);
    R_set_altinteger_No_NA_method(count_seq_class, cs_No_NA);

    live_lgl_class = R_make_altlogical_class("live_lgl", "altrep_pkg", dll);
    R_set_altrep_Length_method(live_lgl_class, ll_Length);
    R_set_altvec_Dataptr_method(live_lgl_class, ll_Dataptr);
    R_set_altlogical_Elt_method(live_lgl_class, ll_Elt);
}
