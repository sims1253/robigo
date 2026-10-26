# compiled forms: builtins, math, &&/||, ranges, computed indices
f <- function(n) {
  x <- numeric(n); s <- 0
  for (i in 1:n) x[i] <- sqrt(i) + abs(-i) + floor(i / 2)
  for (i in seq_len(n - 1)) s <- s + x[i + 1] - x[i]
  for (j in seq_along(x)) if (j %% 2 == 0 && x[j] > 3 || j == 1) s <- s + 1
  for (k in n:1) s <- s + k
  for (k in 0:0) s <- s + 100
  for (k in seq_len(0)) s <- s + 1000
  c(s, length(x), max(x), sum(x))
}
f(10)
g <- function(v) { out <- numeric(length(v)); for (i in 2:length(v)) out[i - 1] <- v[i] * 2; out }
g(c(1, 2, 3, 4))
h <- function() { r <- 0; for (i in 1:3) for (j in i:3) r <- r + i * j; r }
h()
# errors and warnings keep their calls
tryCatch((function() { x <- 1:3; x[[2 + 3]] })(), error = function(e) print(conditionCall(e)))
tryCatch((function() { x <- 1:3; x[[2 + 3]] <- 1:2; x })(), error = function(e) print(conditionCall(e)))
tryCatch((function(a) a && TRUE)("x"), error = function(e) print(conditionMessage(e)))
tryCatch((function(a) TRUE || a)(c(TRUE, FALSE)), error = function(e) print(conditionMessage(e)))
tryCatch((function(a) FALSE || a)(c(TRUE, FALSE)), error = function(e) print(conditionMessage(e)))
(function(a, b) c(a && b, a || b))(NA, FALSE)
(function() { r <- 0; for (i in 1.5:3) r <- r + i; r })()
(function() sqrt(-1))()
(function(x) { y <- x; y[2 + 0] <- 10L; c(x, y) })(1:3)
(function(x) { x[1 + 1] <- 2.5; x })(1:3)
(function() { l <- list(1, "a"); l[[1 + 1]] })()
(function() { v <- c(a = 1, b = 2); v[1 + 1] })()
fac <- factor(c("u", "v")); (function() fac[1 + 1])()
(function() { m <- matrix(1:4, 2); m[2 + 1] })()
`[.myc` <- function(x, i) { cat("method sees", deparse(substitute(i)), "\n"); unclass(x)[i] }
o <- structure(1:5, class = "myc"); (function(k) o[k + 1])(2)
(function() { n <- 3L; for (i in 1:n) n <- n - 1L; n })()
