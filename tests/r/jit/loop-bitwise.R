# bitwAnd and friends in loops: the VM's unboxed fast path, calls of the
# base closures run inline and native loops; NA, out-of-range shifts,
# coercion of doubles (NaN to NA, too large: NA with a warning), errors.
f <- function(n) {
  s <- 0L; a <- 0
  for (i in 1:n) {
    a <- bitwShiftL(a, 1) + (i %% 3 == 0)
    s <- bitwXor(s, a)
    t <- bitwAnd(s, 1023L)
    u <- bitwShiftR(s, 2)
    s <- bitwOr(t, u)
    if (a > 1000) a <- 0
  }
  c(s, a)
}
for (k in 1:30) r <- f(200)
print(r)
print(f(5))
g <- function(x, y) {
  out <- integer(length(x))
  for (i in 1:length(x)) out[i] <- bitwAnd(x[i], y[i])
  out
}
for (k in 1:20) g(1:20, 20:1)
print(g(c(5, NA, 7.9, -1, 2^31 - 1), c(3, 1, 3, 2, 1)))
h <- function(n, s) {
  r <- integer(n)
  for (i in 1:n) r[i] <- bitwShiftL(1L, s + i)
  r
}
for (k in 1:20) h(10, 0)
print(h(10, 20))
print(h(40, -3))
w <- function(n) {
  r <- 0
  big <- 2^31 - 20
  for (i in 1:n) r <- r + bitwAnd(big + i, 1L)
  r
}
print(tryCatch(w(30), warning = function(e) conditionMessage(e)))
print(suppressWarnings(w(30)))
z <- function(n) {
  r <- 0
  for (i in 1:n) { v <- if (i > 25) NaN else i; r <- r + bitwAnd(v, 3) }
  r
}
for (k in 1:5) print(z(30))
e <- function(n) { r <- 0; for (i in 1:n) { v <- if (i < 25) i else TRUE; r <- r + bitwAnd(v, 1L) }; r }
print(tryCatch(e(30), error = function(e) conditionMessage(e)))
print(bitwShiftL(5, 33)); print(bitwShiftR(-1L, 28)); print(tryCatch(bitwXor(NA, 1L), error = function(e) conditionMessage(e)))
