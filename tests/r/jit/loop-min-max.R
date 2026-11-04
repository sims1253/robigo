# `min(a, b)` and `max(a, b)` of two numeric scalars in native loops
# (jit.rs `min_max`): integer for two integers, else double; of equal
# values the first; NA and NaN leave native code for the builtin.
f <- function(n, k) {
  s <- 0
  for (i in 1:n) {
    j <- min(i + 1L, k)
    s <- s + j * 2 + max(i, 3.5) - min(2.5, i) + max(-i, -k)
  }
  s
}
print(f(30, 7L)); print(f(30, 7)); print(f(3, 2L))
ty <- function() {
  r <- list()
  for (i in 1:20) r <- list(min(i, 3L), max(i, 2.5), min(1L, i * 1.0), max(i, i))
  print(lapply(r, typeof)); print(r)
}
ty()
zeros <- function(a, b) {
  for (i in 1:20) { u <- min(a, b); v <- min(b, a); w <- max(a, b); x <- max(b, a) }
  print(1 / c(u, v, w, x))
}
zeros(0, -0)
odd <- function(v) {
  s <- 0
  for (i in 1:20) s <- s + min(v, i) + max(i, v)
  s
}
print(odd(5)); print(odd(NA)); print(odd(NaN)); print(odd(NA_integer_)); print(odd(Inf)); print(odd(-Inf))
print(odd(TRUE)); print(tryCatch(odd("a"), error = function(e) conditionMessage(e)))
mixnan <- function() { r <- 0; for (i in 1:20) r <- min(if (i == 15) NaN else i, NA); r }
print(mixnan())
three <- function() { s <- 0; for (i in 1:20) s <- s + min(i, 5, 3) + max(i, na.rm = TRUE); s }
print(three())
reb <- function() {
  min <- function(a, b) a * 100
  s <- 0
  for (i in 1:20) s <- s + min(i, 1)
  s
}
print(reb())
