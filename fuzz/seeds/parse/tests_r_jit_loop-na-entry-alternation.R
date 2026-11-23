# Each invocation has the same scalar types, but the NA-bearing slot moves.
f <- function(a, b) {
  cat("")
  s <- 0
  for (i in 1:40) s <- s + (a + 1L) + (b + 1L)
  s
}
print(f(1L, 2L))
for (k in 1:4) {
  x <- f(NA_integer_, 2L)
  print(c(is.na(x), is.nan(x)))
  x <- f(1L, NA_integer_)
  print(c(is.na(x), is.nan(x)))
}
print(f(1L, 2L))
