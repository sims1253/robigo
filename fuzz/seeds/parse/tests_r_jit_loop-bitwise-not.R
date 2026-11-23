f <- function(n, x) {
  r <- integer(n)
  for (i in 1:n) r[i] <- bitwNot(x + i)
  r
}
for (k in 1:20) f(40L, 1L)
print(f(40L, 1L))
print(f(40L, NA_integer_))
print(f(40L, NaN))
print(suppressWarnings(f(40L, 2147483620)))
print(f(40L, -2147483640))
g <- function(n) {
  z <- 0i
  for (i in 1:n) { z <- z + (1 + 2i); if (i == 30L) return(list(z, length(z))) }
  z
}
for (k in 1:20) g(40L)
print(g(40L))
# Calls must still resolve their actual binding on each native entry.
h <- function(n) { x <- 1L; for (i in 1:n) x <- bitwNot(x); x }
for (k in 1:20) h(41L)
print(h(41L))
bitwNot <- function(a) 7L
print(h(41L))
rm(bitwNot)
print(h(41L))
# A wrapper that returns an argument must preserve its integer/double tag.
keep <- function(a, b) { .Internal(bitwiseNot(b)); a }
k <- function(n) {
  x <- 0
  for (i in 1:n) { x <- bitwXor(x, i); x <- keep(x, 1L) }
  list(x, typeof(x))
}
for (j in 1:20) k(40L)
print(k(40L))
