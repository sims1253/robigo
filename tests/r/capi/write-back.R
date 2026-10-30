# Exercise native writes without depending on an installed CRAN package.
# Declare the public function ABI directly so both runtimes use their own
# accessors, even when R_HOME points at GNU R's package library.
d <- tempfile(); dir.create(d)
cfile <- file.path(d, "bridge.c")
so <- file.path(d, paste0("bridge", .Platform$dynlib.ext))
writeLines('
typedef void *SEXP;
extern SEXP Rf_protect(SEXP), Rf_duplicate(SEXP), Rf_shallow_duplicate(SEXP);
extern void Rf_unprotect(int);
extern SEXP VECTOR_ELT(SEXP, long), SET_VECTOR_ELT(SEXP, long, SEXP);
extern SEXP Rf_mkChar(const char *), Rf_install(const char *);
extern void SET_STRING_ELT(SEXP, long, SEXP);
extern SEXP Rf_setAttrib(SEXP, SEXP, SEXP), R_do_slot_assign(SEXP, SEXP, SEXP);
extern double *REAL(SEXP);
SEXP replace(SEXP x, SEXP v) { SET_VECTOR_ELT(x, 0, v); return x; }
SEXP alias(SEXP x) { SET_VECTOR_ELT(x, 0, VECTOR_ELT(x, 1)); return x; }
SEXP touch(SEXP x) { REAL(x)[0] = 99; return x; }
SEXP text(SEXP x) { SET_STRING_ELT(x, 0, Rf_mkChar("changed")); return x; }
SEXP attribute(SEXP x, SEXP v) { Rf_setAttrib(x, Rf_install("note"), v); return x; }
SEXP slot(SEXP x, SEXP v) { return R_do_slot_assign(x, Rf_install("x"), v); }
SEXP copy(SEXP x, SEXP deep) {
 SEXP y = Rf_protect(REAL(deep)[0] ? Rf_duplicate(x) : Rf_shallow_duplicate(x));
 REAL(VECTOR_ELT(y, 0))[0] = 99;
 Rf_unprotect(1); return y;
}
', cfile)
stopifnot(system2("cc", c("-shared", "-fPIC", "-o", shQuote(so), shQuote(cfile)), stdout = FALSE, stderr = FALSE) == 0)
dyn.load(so)
x <- list(1, 2); y <- x
invisible(.Call("replace", x, 42))
print(x); print(y)
x <- list(1, 2)
invisible(.Call("alias", x))
invisible(.Call("replace", x, 3))
print(x)
x <- list(c(1, 2), c(3, 4))
invisible(.Call("alias", x))
invisible(.Call("touch", x[[1]]))
print(x)
x <- "original"; y <- x
invisible(.Call("text", x))
print(x); print(y)
x <- matrix(1:4, 2); y <- x
invisible(.Call("attribute", x, "native"))
print(attr(x, "note")); print(attr(y, "note"))
x <- list(c(1, 2)); y <- .Call("copy", x, 1)
print(x); print(y)
y <- .Call("copy", x, 0)
print(x); print(y)
library(methods)
setClass("BridgeWrite", slots = c(x = "numeric"))
x <- new("BridgeWrite", x = 1); y <- x
invisible(.Call("slot", x, 7))
print(x@x); print(y@x)
dyn.unload(so)
unlink(d, recursive = TRUE)
