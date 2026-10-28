// C API microbenchmarks (council item 9): each bench_* routine runs one
// API operation N times; bench.py counts the instructions inside each
// routine under callgrind (--toggle-collect), for robigo and GNU R.
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Rdynload.h>

#define N 100000

SEXP bench_alloc_small(void) {
    for (int i = 0; i < N; i++) allocVector(REALSXP, 4);
    return R_NilValue;
}

SEXP bench_alloc_medium(void) {
    for (int i = 0; i < N / 10; i++) allocVector(REALSXP, 1000);
    return R_NilValue;
}

SEXP bench_cons(void) {
    SEXP s = install("f");
    for (int i = 0; i < N; i++) lang3(s, R_NilValue, R_NilValue);
    return R_NilValue;
}

SEXP bench_protect(void) {
    SEXP x = allocVector(INTSXP, 1);
    for (int i = 0; i < N; i++) { PROTECT(x); PROTECT(x); UNPROTECT(2); }
    return R_NilValue;
}

SEXP bench_mkchar_hit(void) {
    for (int i = 0; i < N; i++) mkChar("hello world");
    return R_NilValue;
}

SEXP bench_mkchar_miss(void) {
    char buf[32];
    for (int i = 0; i < N; i++) { snprintf(buf, 32, "s%d", i); mkChar(buf); }
    return R_NilValue;
}

SEXP bench_install(void) {
    for (int i = 0; i < N; i++) install("bench_symbol");
    return R_NilValue;
}

SEXP bench_integer_loop(SEXP x) {
    double s = 0;
    for (int k = 0; k < 10; k++) {
        R_xlen_t n = XLENGTH(x);
        for (R_xlen_t i = 0; i < n; i++) s += INTEGER(x)[i];
    }
    return ScalarReal(s);
}

SEXP bench_set_string_elt(void) {
    SEXP v = PROTECT(allocVector(STRSXP, 100));
    SEXP c = PROTECT(mkChar("x"));
    for (int i = 0; i < N; i++) SET_STRING_ELT(v, i % 100, c);
    UNPROTECT(2);
    return R_NilValue;
}

SEXP bench_preserve(void) {
    for (int i = 0; i < N / 10; i++) {
        SEXP x = allocVector(INTSXP, 1);
        R_PreserveObject(x);
        R_ReleaseObject(x);
    }
    return R_NilValue;
}

SEXP bench_eval(SEXP f) {
    SEXP call = PROTECT(lang1(f));
    for (int i = 0; i < N / 10; i++) eval(call, R_GlobalEnv);
    UNPROTECT(1);
    return R_NilValue;
}

static SEXP body_ok(void *d) { return R_NilValue; }
static void cleanup(void *d, Rboolean j) {}

SEXP bench_unwind_protect(void) {
    SEXP cont = PROTECT(R_MakeUnwindCont());
    for (int i = 0; i < N / 10; i++) R_UnwindProtect(body_ok, NULL, cleanup, NULL, cont);
    UNPROTECT(1);
    return R_NilValue;
}

static SEXP body_err(void *d) { error("boom"); return R_NilValue; }

SEXP bench_error_caught(void) {
    SEXP cont = PROTECT(R_MakeUnwindCont());
    for (int i = 0; i < N / 100; i++) {
        R_UnwindProtect(body_err, NULL, cleanup, NULL, cont);
    }
    UNPROTECT(1);
    return R_NilValue;
}

SEXP bench_noop(SEXP a, SEXP b, SEXP c) { return R_NilValue; }
