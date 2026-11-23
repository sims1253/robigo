# Arithmetic and comparisons with operands read in place (compiled code)
side <- function(tag, v) { cat("force", tag, "\n"); v }
ord <- function(a, b) a + b
ord(side("a", 1), side("b", 2))
ord2 <- function(a, b) { x <- a; x * b }
ord2(side("x", 3), side("y", 4))
lt <- function(a, b) a < b
lt(side("p", 1L), side("q", 2))
f <- function() { big <- .Machine$integer.max; big + 1L }
f()
g <- function(n) { s <- 0L; for (i in 1:n) s <- s + i; s }
g(10)
h <- function() { x <- NA_integer_; c(x + 1L, x > 1, 5L %/% 0L, 5L %% 0L, -7 %/% 2, -7 %% 2, 2^10, 0L / 0L) }
h()
fac <- factor(c("a", "b"))
cmpf <- function(z) z == "a"
cmpf(fac)
tf <- function() c(T + 1, F * 2, TRUE + TRUE)
tf()
missing_var <- function() undefined_variable_xyz + 1
tryCatch(missing_var(), error = function(e) conditionMessage(e))
strcmp <- function(a, b) a < b
strcmp("apple", "banana")
local({
  `+` <- function(e1, e2) paste("plus", e1, e2)
  k <- function(v) v + 1
  print(k(2))
})
k2 <- function(v) v + 1
k2(2)
w <- function() { x <- 1:3; x + 1 }
w()
loopsum <- function() {
  acc <- 0
  for (i in 1:5) acc <- acc + i * 2 - 1
  acc
}
loopsum()
warn_div <- function() { a <- 1L; b <- 0L; c(a / b, a %/% b) }
warn_div()
