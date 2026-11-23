f <- function(a, b) c(missing(a), missing(b))
f()
f(1)
f(b = 2)
g <- function(x = 5) missing(x)
g()
g(5)
h <- function(...) nargs()
h()
h(1, 2, 3)
k <- function(...) ...length()
k(1, "a", NULL)
m <- function(...) ..2
m("a", "b", "c")
n <- function(...) names(list(...))
n(a = 1, 2, b = 3)
p <- function(first, ...) {
  inner <- function(...) sum(...)
  inner(...)
}
p(100, 1, 2, 3)
q <- function(x) {
  force(x)
  function() x
}
q(42)()
lazy <- function(a, b) {
  if (a) "only a" else b
}
lazy(TRUE, stop("never evaluated"))
