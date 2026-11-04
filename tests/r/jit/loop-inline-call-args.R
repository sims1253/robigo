# A loop in native code calls a pure closure inline with arguments that
# are arithmetic on, or element reads of, the caller's variables
# (jit.rs CallArg, vm::inline_call). Native code evaluates them before
# the call where they take their fast paths; anything else (a warning, an
# error, an argument the callee never uses) is the VM's lazy promise.
f <- function(i, j) 1 / ((i + j) * (i + j + 1) / 2 + i + 1)
a1 <- function(n) {
  s <- 0
  for (i in 1:n) for (j in 0:(n - 1)) s <- s + f(i - 1, j)
  s
}
print(a1(30)); print(a1(31))
a2 <- function(n) {
  s <- 0L
  g <- function(a, b) a * b
  for (i in 1:n) s <- s + g(i - 1L, i + 2L)
  s
}
print(a2(100))
# element reads, `[[` and `[`
a3 <- function(u) {
  r <- 0
  h <- function(x, k) x * k
  for (j in seq_along(u)) r <- r + h(u[[j]], j) + h(u[j], 2)
  r
}
print(a3(c(1.5, 2, 3, 4.25, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20)))
print(a3(1:20))
# the vector changes inside the loop
a4 <- function(n) {
  u <- rep(1, n)
  h <- function(x, y) x + y
  for (j in 2:n) u[[j]] <- h(u[[j - 1]], j)
  u
}
print(a4(25))
# integer overflow in an argument: NA and the warning, from the promise
big <- .Machine$integer.max - 10L
a5 <- function() {
  g <- function(a) a
  r <- 0L
  for (i in 1:20) r <- g(big + i)
  r
}
print(a5())
# an argument the callee never uses is never evaluated
a6 <- function(u) {
  g <- function(a, b) a
  r <- 0
  for (i in 1:30) r <- r + g(i, u[[i + 100]]) + g(1, big + i)
  r
}
print(a6(c(1, 2, 3)))
# a used one that fails: R's error
a7 <- function(u) {
  g <- function(a, b) a + b
  r <- 0
  for (i in 1:30) r <- r + g(i, u[[i]])
  r
}
print(tryCatch(a7(as.numeric(1:20)), error = function(e) conditionMessage(e)))
a8 <- function(u) {
  g <- function(a, b) a + b
  r <- 0
  for (i in 1:30) r <- r + g(i, u[i])
  r
}
print(a8(as.numeric(1:20)))
# NA operands
a9 <- function(x) {
  g <- function(a) a
  r <- 0
  for (i in 1:20) r <- g(x - i)
  r
}
print(a9(NA_integer_)); print(a9(NA_real_)); print(a9(5L))
# `-` and `[[` rebound where the loop runs
a10 <- function(n) {
  `-` <- function(e1, e2) base::`+`(e1, e2)
  g <- function(a) a
  s <- 0
  for (i in 1:n) s <- s + g(i - 1)
  s
}
print(a10(20))
a11 <- function(u) {
  `[[` <- function(x, i) 100
  g <- function(a) a
  s <- 0
  for (i in seq_along(u)) s <- s + g(u[[i]])
  s
}
print(a11(as.numeric(1:20)))
# rebound in the global environment between calls
a12 <- function(n) {
  g <- function(a) a
  s <- 0
  for (i in 1:n) s <- s + g(i * 2)
  s
}
print(a12(20)); print(a12(20))
`*` <- function(e1, e2) base::`*`(e1, e2) + 1000
print(a12(20))
rm(`*`)
print(a12(20))
# an argument variable that is not numeric
a13 <- function(x) {
  g <- function(a) a
  r <- NULL
  for (i in 1:20) r <- g(x[[i]])
  r
}
print(a13(letters)); print(a13(as.list(1:26)))
