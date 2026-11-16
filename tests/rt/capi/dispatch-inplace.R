# A method's write to its argument, seen by the native observer of in-place
# writes (support/identity.c's rt_assign_copies: TRUE when the write
# copies). GNU R's method shares the generic's promise (dispatchMethod
# passes cptr->promargs), so a value only that promise references is
# written in place, as in a closure called directly; one a variable also
# references is copied. ROBIGO_NO_INPLACE=1 copies at every write, so this
# recording holds in production only (the .requires: inplace-identity).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/identity.so"))
copies <- function(e) .Call("rt_assign_copies", quote(x), e, parent.frame())
f <- function(x, ...) UseMethod("f")
f.default <- function(x, ...) copies(quote(x[1] <- 9))
f.foo <- function(x, ...) copies(quote(x[1] <- 9))
# a fresh value, at top level
print(f(c(1, 2, 3)))
# a variable's value: it and the promise reference it
g <- function() {
  v <- c(1, 2, 3)
  f(v)
}
print(g())
# a fresh value, in a compiled loop (its first calls run the AST)
h <- function() {
  r <- logical(3)
  for (i in 1:3) r[i] <- f(c(1, 2, 3))
  r
}
print(h())
print(h())
# a classed object, in a compiled loop
k <- function() {
  r <- logical(3)
  for (i in 1:3) r[i] <- f(structure(c(1, 2, 3), class = "foo"))
  r
}
print(k())
print(k())
# through NextMethod
f.bar <- function(x, ...) NextMethod()
print(f(structure(c(1, 2, 3), class = "bar")))
# the generic sees the method's write: one promise
gw <- function(x, ...) {
  on.exit(print(x))
  UseMethod("gw")
}
gw.default <- function(x, ...) {
  x[1] <- 9
  invisible()
}
gw(c(1, 2, 3))
# a closure called directly, for comparison
d <- function(x) copies(quote(x[1] <- 9))
print(d(c(1, 2, 3)))
