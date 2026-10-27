# Guards of closures defined in a call: shared memo, invalidation
outer <- function() {
  f <- function(v) v + 1
  r1 <- vapply(1:3, f, 0)
  `+` <- function(a, b) a * b
  r2 <- vapply(1:3, f, 0)
  list(r1, r2)
}
outer()
loop <- function() {
  g <- function(v) if (v > 1) v - 1 else v
  s <- 0
  for (k in 1:30) s <- s + g(k)
  `-` <- function(a, b) 100
  s2 <- 0
  for (k in 1:3) s2 <- s2 + g(k)
  c(s, s2)
}
loop()
pe <- function() {
  h <- function(x) x + 1
  r1 <- h(1)
  e <- new.env(parent = globalenv())
  assign("+", function(a, b) "custom", envir = e)
  parent.env(environment()) <- e
  r2 <- h(1)
  c(r1, r2)
}
pe()
mk <- function(k) function(v) v * k
fs <- lapply(1:3, mk)
vapply(fs, function(f) f(2), 0)
