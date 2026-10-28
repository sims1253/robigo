# Calls of functions found in local frames from loops (helpers defined in
# the enclosing function, arguments, shadowing values, redefinitions).
outer <- function(n) {
  sq <- function(x) x * x
  inner <- function() {
    s <- 0
    for (i in 1:n) s <- s + sq(i)
    s
  }
  inner()
}
print(outer(10))
shadow <- function() {
  sq <- function(x) -x
  g <- function() {
    sq <- 3  # not a function: the lookup goes on to the enclosing frame
    r <- 0
    for (i in 1:3) r <- r + sq(i)
    r
  }
  g()
}
print(shadow())
apply_n <- function(f, n) {
  r <- numeric(n)
  for (i in 1:n) r[i] <- f(i)
  r
}
print(apply_n(function(x) x + 0.5, 4))
print(apply_n(sqrt, 4))
lazy <- function(f) {
  cat("before\n")
  for (i in 1:2) print(f(i))
}
lazy({ cat("forcing f\n"); function(x) x * 10 })
redef <- function() {
  h <- function(x) x + 1
  r <- c()
  for (i in 1:4) {
    r <- c(r, h(i))
    if (i == 2) h <- function(x) x * 100
  }
  r
}
print(redef())
miss <- function(fn) {
  tryCatch(for (i in 1:2) fn(i), error = function(e) print(conditionMessage(e)))
}
miss()
act <- function() {
  e <- environment()
  k <- 0
  makeActiveBinding("af", function() { k <<- k + 1; function(x) x + k }, e)
  r <- 0
  for (i in 1:3) r <- r + af(i)
  c(r, k)
}
print(act())
notfun <- function() {
  vv <- 1
  tryCatch(for (i in 1:2) vv(i), error = function(e) print(conditionMessage(e)))
}
notfun()
rec <- function(n) {
  fib <- function(k) if (k < 2) k else fib(k - 1) + fib(k - 2)
  s <- 0
  for (i in 0:n) s <- s + fib(i)
  s
}
print(rec(15))
warn <- function() {
  lg <- function(x) log(x)
  r <- 0
  for (i in c(1, -1)) r <- r + lg(i)
  r
}
print(warn())
