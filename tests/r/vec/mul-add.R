# `a * b + c`, `a * b - c`, `c + a * b`, `c - a * b` on vectors in compiled
# code (closure bodies and loops): one pass, but the same values, warnings
# and attributes as the two operations.
hex <- function(v) cat(sprintf("%a", v), "\n")
f1 <- function(a, b, c) a * b + c
f2 <- function(a, b, c) a * b - c
f3 <- function(a, b, c) c + a * b
f4 <- function(a, b, c) c - a * b
all4 <- function(a, b, c) {
  print(f1(a, b, c)); print(f2(a, b, c)); print(f3(a, b, c)); print(f4(a, b, c))
}
x <- c(1.5, -2, 3.25, 1e308)
y <- c(2, 0.5, -4, 10)
z <- c(0.25, 1, -1, -Inf)
all4(x, y, z)
all4(x, 2, z)
all4(3, y, z)
all4(x, y, 1)
# NA and NaN, one per element (with two, which survives is up to the
# platform).
a <- c(NA, NaN, 1, 1, 2, 3, Inf, 0)
b <- c(1, 2, NA, NaN, 1, 1, 2, -Inf)
cc <- c(1, 2, 3, 4, NA, NaN, -1, 5)
all4(a, b, cc)
# Signed zeros.
hex(f1(c(-0, 0, -1), c(1, -1, 0), c(-0, -0, 0)))
hex(f2(c(-0, 0, -1), c(1, -1, 0), c(0, 0, -0)))
hex(f4(c(0, -0), c(1, 1), c(0, -0)))
# Rounded product, then the addition (no fused multiply-add).
p <- 1 + 2^-27
hex(f2(c(p, p), c(p, p), c(1 + 2^-26, 1)))
hex(f1(c(0.1, 1/3), c(10, 3), c(-1, -1)))
# Attributes, integers, lengths that do not match, zero length.
print(f1(c(u = 1, v = 2), c(3, 4), c(5, 6)))
print(f1(c(1, 2), c(3, 4), c(p = 5, q = 6)))
print(f1(matrix(1:4 + 0.5, 2), 2, 1))
print(f1(1:3, 2L, 1L)); print(f1(1:3, 2, 1L)); print(f1(c(1, 2, 3), 2, 1L))
print(.Machine$integer.max * 2L + 1L)
print(f1(c(1, 2, 3, 4), c(1, 2), c(1, 2, 3, 4)))
print(f1(c(1, 2, 3), c(1, 2), c(1, 2, 3)))
print(f1(c(1, 2), c(1, 2), c(1, 2, 3)))
print(f1(numeric(0), 2, 3)); print(f1(c(1, 2), 2, numeric(0)))
print(f1(c(TRUE, FALSE), 2, 1))
# The operands are unchanged (the result may reuse a temporary only).
g <- function(x) {
  y <- x * 2 + x
  z <- x - x * x
  w <- sqrt(x) * exp(-x) + log1p(x)
  v <- (x + 1) * (x - 1) - x * 0.5
  list(x, y, z, w, v)
}
xs <- c(1, 4, 9, 16)
print(g(xs)); print(xs)
# In loops.
h <- function(n) {
  s <- c(0, 0, 0)
  k <- c(1, 2, 3)
  for (i in seq_len(n)) s <- s * 0.5 + k
  for (i in seq_len(n)) s <- k - s * 0.25
  s
}
print(h(20))
# Objects dispatch as before.
Ops.money <- function(e1, e2) {
  cat("Ops.money", .Generic, "\n")
  structure(get(.Generic)(unclass(e1), unclass(e2)), class = "money")
}
m <- structure(c(1, 2), class = "money")
print(unclass(f1(m, c(3, 4), c(5, 6))))
print(unclass(f3(c(3, 4), c(5, 6), m)))
# A rebound `+` after the product is still called.
k1 <- function(a, b, c) { `+` <- function(e1, e2) { cat("local +\n"); base::`+`(e1, e2) }; a * b + c }
print(k1(c(1, 2), c(3, 4), c(5, 6)))
