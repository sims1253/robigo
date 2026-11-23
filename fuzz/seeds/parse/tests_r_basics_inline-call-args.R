# Calls of pure closures whose arguments are arithmetic on variables.
f <- function(a, b) a * 10 + b
s <- 0
for (i in 1:5) s <- s + f(i - 1, i * 2)
print(s)
g <- function(a) a + 1L
big <- .Machine$integer.max
r <- NULL
for (k in 0:1) r <- c(r, g(big - k))
print(r)
r <- NULL
for (k in 1:2) r <- c(r, f(big + k, 1L))
print(r)
h <- function() {
  `-` <- function(x, y) x + y
  out <- 0
  for (i in 1:3) out <- out + f(i - 1, 0)
  out
}
print(h())
m <- function() {
  r <- 0
  for (i in 1:3) r <- r + f(i %% 2, i %/% 2)
  r
}
print(m())
w <- function(x) x
for (i in 1:2) print(w(i %% 0))
for (i in 1:2) print(w(i %/% 0L))
for (i in 1:2) print(w(5L %% (i - 1L)))
for (x in c(2.5, NA, -1)) print(f(x / 2, x ^ 2))
lazy_unused <- function(a, b) a
for (i in 1:2) print(lazy_unused(i, stop("not forced")))
t2 <- function(x, y) x == y
for (i in 1:2) print(t2(i + 0.5, 1.5))
lg <- function(a, b) a
for (i in c(TRUE, FALSE)) print(lg(i + i, 0))
