# Integer arithmetic in native loops where operands are known not to be NA
# (loop variables, constants, sums of them) and where they may be: NA
# from a vector, an NA constant, an NA at entry; overflow still warns.
conv <- function(a, b) {
  na <- length(a); nb <- length(b)
  ab <- double(na + nb)
  for (i in 1:na) for (j in 1:nb) ab[i + j] <- ab[i + j] + a[i] * b[j]
  ab
}
for (k in 1:5) r <- conv(as.double(1:30), as.double(30:1))
print(sum(r)); print(r[c(2, 31, 60)])
f <- function(v, start) {
  s <- start; c <- 0L
  for (i in seq_along(v)) {
    x <- v[i]
    s <- s + x * 2L
    if (i > 3L) c <- c + (i < 10L)
    if (i == 7L) s <- s - 1L
  }
  c(s, c)
}
for (k in 1:20) f(1:20, 0L)
print(f(1:20, 0L)); print(f(c(1:5, NA, 7:20), 0L)); print(f(1:20, NA_integer_))
g <- function(n, z) {
  t <- 0L; u <- 1L
  for (i in 1:n) {
    t <- t + i
    if (i == 5L) u <- z
    u <- u + 1L
    if (u > 3L) t <- t - 1L
  }
  c(t, u)
}
for (k in 1:20) g(30, 2L)
print(g(30, 2L)); print(tryCatch(g(30, NA_integer_), error = function(e) conditionMessage(e)))
h <- function(n) { s <- 0L; for (i in 1:n) s <- s + i * 100000L; s }
for (k in 1:20) h(20)
print(h(20)); print(suppressWarnings(h(300)))
print(tryCatch(h(300), warning = function(w) conditionMessage(w)))
m <- function(n, k0) { k <- k0; s <- 0; for (i in 1:n) { s <- s + (i + k); k <- k + 1L }; c(s, k) }
for (k in 1:20) m(20, 1L)
print(m(20, 1L)); print(m(20, NA_integer_)); print(m(20, 2147483600L))
# Helpers run inline: arguments known not to be NA, and ones that may be.
ev <- function(i, j) 1 / ((i + j) * (i + j + 1) / 2 + i + 1)
sp <- function(n, v) {
  out <- double(n)
  for (i in 1:n) {
    k <- v[i]
    out[i] <- ev(i, i - 1L) + ev(k, 2L) + ev(i + 1L, k)
  }
  out
}
for (r in 1:20) sp(20, 1:20)
print(sum(sp(20, 1:20)), digits = 15)
print(sp(6, c(1L, 2L, NA, 4L, 5L, 6L)))
iv <- function(a, b) { if (a > b) a - b else b - a }
dd <- function(n, z) { s <- 0L; for (i in 1:n) s <- s + iv(i, z); s }
for (r in 1:20) dd(20, 5L)
print(dd(20, 5L)); print(tryCatch(dd(20, NA_integer_), error = function(e) conditionMessage(e)))
