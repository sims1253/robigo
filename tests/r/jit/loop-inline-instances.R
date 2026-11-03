# A loop in native code calls a pure closure inline (jit.rs, vm::jit_enter).
# Another instance of the same closure (one made by each call of the
# enclosing function) runs there too; each instance's own environment
# decides what its functions are.
outer <- function(n, k) {
  f <- function(i, j) i * k + j
  s <- 0
  for (i in 1:n) for (j in 1:n) s <- s + f(i, j)
  s
}
print(outer(50, 2)); print(outer(50, 3)); print(outer(10, 0.5))
for (r in 1:5) x <- outer(30, r)
print(x)
# an instance made where `+` is rebound
mk <- function(plus) {
  if (plus) `+` <- function(e1, e2) base::`+`(e1, e2) * 10
  f <- function(i, j) i + j
  s <- 0
  for (i in 1:40) s <- s + f(i, 1)
  s
}
print(mk(FALSE)); print(mk(TRUE)); print(mk(FALSE))
# the instance changes inside the loop
ch <- function() {
  s <- 0
  for (i in 1:100) {
    h <- if (i %% 2 == 0) function(x) x + 1 else function(x) x + 1
    s <- s + h(i)
  }
  s
}
print(ch())
# instances of different bodies with the same formals
db <- function(which) {
  h <- if (which) function(x) x * 3 else function(x) x - 3
  s <- 0
  for (i in 1:60) s <- s + h(i)
  s
}
print(db(TRUE)); print(db(FALSE)); print(db(TRUE))
