f <- function(x, y = 2) x + y
f(1)
f(1, 3)
f(y = 10, x = 1)
g <- function(...) sum(...)
g(1, 2, 3)
h <- function(a, b) {
  if (missing(b)) "no b" else b
}
h(1)
h(1, "yes")
compose <- function(f, g) function(x) g(f(x))
inc <- function(x) x + 1
dbl <- function(x) x * 2
compose(inc, dbl)(3)
make_counter <- function() {
  i <- 0
  function() {
    i <<- i + 1
    i
  }
}
cnt <- make_counter()
cnt()
cnt()
cnt()
fact <- function(n) if (n <= 1) 1 else n * fact(n - 1)
fact(10)
(function(x) x^2)(4)
sq <- \(x) x^2
sq(5)
args_test <- function(first, ...) {
  rest <- list(...)
  length(rest)
}
args_test(1, 2, 3, 4)
partial <- function(alpha, beta) alpha - beta
partial(be = 1, al = 10)
do.call(f, list(1, 2))
do.call("paste", list("a", "b", sep = "-"))
Recall
f2 <- function(n) if (n == 0) 0 else n + Recall(n - 1)
f2(4)
