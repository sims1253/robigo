# A native routine called on the object the evaluator is walking: passing it
# must not move the object's cells, whether or not the routine changes them.
d <- tempfile(); dir.create(d)
cfile <- file.path(d, "running.c")
so <- file.path(d, paste0("running", .Platform$dynlib.ext))
writeLines('
typedef void *SEXP;
extern SEXP R_NilValue;
extern SEXP SET_VECTOR_ELT(SEXP, long, SEXP);
SEXP noop(SEXP x) { return R_NilValue; }
SEXP store(SEXP x, SEXP v) { SET_VECTOR_ELT(x, 2, v); return R_NilValue; }
', cfile)
stopifnot(system2("cc", c("-shared", "-fPIC", "-o", shQuote(so), shQuote(cfile)), stdout = FALSE, stderr = FALSE) == 0)
dyn.load(so)

# what the freed buffer gets reused for
churn <- function(n) keep <<- lapply(1:64, function(i) rep(1.5, n))

h <- function(z, n) { .Call("noop", z); churn(n); 1 }
ex <- expression(h(ex, 8), 2, 3, 4, 5, 6, 7, 8)
for (n in c(4, 8, 12, 15, 16, 20, 24, 30, 32)) {
  ex[[1]] <- substitute(h(ex, N), list(N = n))
  print(eval(ex))
}

cl <- quote(c(h(cl, 8), 2, 3, 4, 5, 6, 7, 8))
for (n in c(4, 8, 12, 15, 16, 20, 24, 30, 32)) {
  cl[[2]] <- substitute(h(cl, N), list(N = n))
  print(eval(cl))
}

# attributes survive, and with them the object's identity
x <- structure(list(a = 1, b = "two"), note = "kept")
invisible(.Call("noop", x))
str <- structure(c(u = "café", v = "x"), note = "kept")
invisible(.Call("noop", str))
print(x); print(str)

# a real write to a cell still ahead of the walk: GNU R re-reads the element
s <- function(z) { .Call("store", z, quote(10 + 1)); 0 }
ex <- expression(s(ex), 2, 3, 4)
print(eval(ex))
ex <- expression(s(ex), 2, "not reached yet")
print(eval(ex))
print(ex[[3]])

dyn.unload(so)
unlink(d, recursive = TRUE)
