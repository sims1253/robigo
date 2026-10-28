// GC at scale (council item 9): a large live heap of unique strings, then
// allocation churn. scale.R reports wall time, time in collection and
// peak RSS for GNU R and robigo.
#include <R.h>
#include <Rinternals.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

static double rss_mb(void) {
    FILE *f = fopen("/proc/self/status", "r");
    char line[256];
    double kb = 0;
    while (f && fgets(line, sizeof line, f))
        if (strncmp(line, "VmRSS:", 6) == 0) kb = atof(line + 6);
    if (f) fclose(f);
    return kb / 1024;
}
static double rss_built;

extern double robigo_gc_seconds(void) __attribute__((weak));
extern double robigo_gc_count(void) __attribute__((weak));

SEXP scale_live(SEXP sn) {
    int n = asInteger(sn);
    SEXP v = PROTECT(allocVector(STRSXP, n));
    char buf[32];
    for (int i = 0; i < n; i++) {
        snprintf(buf, 32, "live-%d", i);
        SET_STRING_ELT(v, i, mkChar(buf));
    }
    rss_built = rss_mb();
    UNPROTECT(1);
    return v;
}

SEXP scale_churn(SEXP sn) {
    double n = asReal(sn);
    for (double i = 0; i < n; i++) allocVector(REALSXP, 4);
    return R_NilValue;
}

SEXP scale_gc_seconds(void) {
    return ScalarReal(robigo_gc_seconds ? robigo_gc_seconds() : NA_REAL);
}

SEXP scale_gc_count(void) {
    return ScalarReal(robigo_gc_count ? robigo_gc_count() : NA_REAL);
}

SEXP scale_rss(void) {
    SEXP r = allocVector(REALSXP, 2);
    REAL(r)[0] = rss_built;
    REAL(r)[1] = rss_mb();
    return r;
}
