# Internal dispatch from a primitive or internal generic runs the method
# inside a function context for the generic's call (R: eval.c:
# DispatchOrEval, begincontext(CTXT_RETURN, call, rho1, rho, pargs, op)).
# sys.calls(), sys.function(), sys.nframe() and sys.parents() see it.
`[.foo` <- function(x, i) {
  print(sys.nframe())
  print(sys.calls())
  print(sys.function(1))
  print(sys.parent())
  print(sys.parents())
  print(identical(parent.frame(), globalenv()))
  invisible()
}
x <- structure(1, class = "foo")
x[1]
g <- function() x[1]
g()
`[<-.foo` <- function(x, i, value) {
  print(sys.calls())
  print(sys.parents())
  x
}
x[1] <- 2
length.foo <- function(x) {
  print(sys.calls())
  1L
}
length(x)
as.character.foo <- function(x, ...) {
  print(sys.calls())
  "a"
}
as.character(x)
# a closure generic has no such context
print.foo <- function(x, ...) {
  print(sys.calls())
  invisible(x)
}
print(x)
# the frame below an S3 method is the generic's call
`$.foo` <- function(x, name) {
  k <- sys.nframe()
  print(sys.call(k - 1L))
  print(sys.function(k - 1L))
  NULL
}
x$a
# parent.frame() of the method is still the caller
h <- function() {
  `[[.foo` <- function(x, i) parent.frame()
  e <- environment()
  identical(x[[1]], e)
}
print(h())
