# A package ALTREP class whose Length method allocates and re-enters R
# (docs/g3.md §8, §15.3 E6): `length()` and `seq_along()` call the method
# where and as often as GNU R calls it (ALTREP_LENGTH: `length()` twice,
# `xlength` then `length`, and answers with the second); the method's R
# code runs a collection, changes a global and can fail; a class without
# the method has GNU R's default, an error.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/altrep_length.so"))
invisible(.Call("rt_altrep_init"))
n <- 0L
lenfun <- function() {
  n <<- n + 1L
  invisible(gc())
  3L + n
}
x <- .Call("rt_altrep_new", lenfun)
print(typeof(x))
print(.Call("rt_altrep_info", x))
print(.Call("rt_altrep_info", c(1L, 2L)))
print(length(x))
print(length(x))
print(c(n, .Call("rt_altrep_calls")))
print(seq_along(x))
# from a closure, as the AST and as byte code
f <- function(v) length(v)
print(f(x)); print(f(x)); print(f(x))
print(c(n, .Call("rt_altrep_calls")))
# the method's R code fails: the error leaves through the C method
bad <- .Call("rt_altrep_new", function() stop("no length today"))
print(tryCatch(length(bad), error = conditionMessage))
print(length(x))
# no Length method
z <- .Call("rt_altrep_no_length")
print(tryCatch(length(z), error = conditionMessage))
print(c(n, .Call("rt_altrep_calls")))
