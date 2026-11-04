# Native code under gctorture: the C heap collects on every allocation, so
# an object the code or the bridge forgets to root is freed (and poisoned)
# at once. Declares the public ABI directly (no headers), as write-back.R.
d <- tempfile(); dir.create(d)
cfile <- file.path(d, "torture.c")
so <- file.path(d, paste0("torture", .Platform$dynlib.ext))
writeLines('
#include <stdio.h>
typedef void *SEXP;
extern SEXP R_NilValue;
extern SEXP Rf_protect(SEXP);
extern void Rf_unprotect(int);
extern SEXP Rf_allocVector(unsigned int, long);
extern SEXP Rf_mkChar(const char *), Rf_install(const char *);
extern SEXP Rf_cons(SEXP, SEXP), Rf_lang2(SEXP, SEXP), Rf_eval(SEXP, SEXP);
extern SEXP Rf_ScalarReal(double);
extern SEXP Rf_setAttrib(SEXP, SEXP, SEXP);
extern SEXP VECTOR_ELT(SEXP, long), SET_VECTOR_ELT(SEXP, long, SEXP);
extern void SET_STRING_ELT(SEXP, long, SEXP);
extern int *INTEGER(SEXP), Rf_length(SEXP);
SEXP build(SEXP n_) {
    int n = INTEGER(n_)[0];
    SEXP out = Rf_protect(Rf_allocVector(19, n));
    SEXP nms = Rf_protect(Rf_allocVector(16, n));
    for (int i = 0; i < n; i++) {
        char buf[32];
        SEXP v = Rf_protect(Rf_allocVector(13, i + 1));
        for (int j = 0; j <= i; j++) INTEGER(v)[j] = i * j;
        SET_VECTOR_ELT(out, i, v);
        Rf_unprotect(1);
        snprintf(buf, sizeof buf, "e%d", i);
        SET_STRING_ELT(nms, i, Rf_mkChar(buf));
    }
    Rf_setAttrib(out, Rf_install("names"), nms);
    Rf_unprotect(2);
    return out;
}
SEXP chain(SEXP n_) {
    SEXP l = Rf_protect(R_NilValue);
    for (int i = INTEGER(n_)[0]; i > 0; i--) {
        SEXP x = Rf_protect(Rf_ScalarReal(i / 2.0));
        l = Rf_cons(x, l);
        Rf_unprotect(2);
        Rf_protect(l);
    }
    Rf_unprotect(1);
    return l;
}
SEXP map(SEXP f, SEXP x, SEXP rho) {
    int n = Rf_length(x);
    SEXP out = Rf_protect(Rf_allocVector(19, n));
    for (int i = 0; i < n; i++) {
        SEXP call = Rf_protect(Rf_lang2(f, VECTOR_ELT(x, i)));
        SET_VECTOR_ELT(out, i, Rf_eval(call, rho));
        Rf_unprotect(1);
    }
    Rf_unprotect(1);
    return out;
}
', cfile)
stopifnot(system2("cc", c("-shared", "-fPIC", "-o", shQuote(so), shQuote(cfile)), stdout = FALSE, stderr = FALSE) == 0)
dyn.load(so)
f <- function(v) list(sum(v), rev(v)[1:2], paste0("s", length(v)))
old <- gctorture(TRUE)
b <- .Call("build", 25L)
l <- .Call("chain", 20L)
m <- .Call("map", f, b, environment())
on <- gctorture(FALSE)
print(c(old, on))
print(names(b)[c(1, 2, 25)])
print(b[[25]][20:25])
print(unlist(l))
print(m[[25]])
print(sapply(m, function(r) r[[1]])[1:10])
dyn.unload(so)
