# Arguments a caller may evaluate itself (the callee forces them first
# thing): values, expressions, laziness and errors stay R's.
fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)
print(fib(15))
expr_of <- function(n) if (n > 0) substitute(n) else NULL
x <- 3
for (i in 1:2) print(expr_of(x - i))
miss <- function(n) if (n > 0) missing(n) else NA
for (i in 1:2) print(miss(i + 0))
# The callee changes the caller's variable before forcing: stays lazy.
late <- function(n) { assign("x", 100, envir = parent.frame()); n }
f <- function() { x <- 1; r <- 0; for (i in 1:2) r <- r + late(x - 1); r }
print(f())
# A rebound comparison in the caller's scope: the body's guards fail.
g <- function(n) if (n < 2) "small" else "big"
h <- function() {
  `<` <- function(a, b) { cat("my <\n"); TRUE }
  r <- character()
  for (i in 3:4) r <- c(r, g(i - 1))
  r
}
print(h())
# Integer overflow in the argument: lazy, warns where R does.
big <- .Machine$integer.max
w <- function(n) if (n > 0) "pos" else "other"
for (k in 0:1) print(tryCatch(w(big + k), error = conditionMessage))
# An argument that errors is never evaluated early.
for (i in 1:2) print(tryCatch(w(stop("boom")), error = conditionMessage))
# sys.call and match.call see the expressions.
sc <- function(a, b) if (a > b) sys.call() else match.call()
for (i in 1:2) print(sc(i * 2, 3))
# Promises shared with a frame that outlives the call.
mk <- function(n) { if (n > 0) function() n else NULL }
fs <- list()
for (i in 1:3) fs[[i]] <- mk(i * 10)
print(sapply(fs, function(f) f()))
# Logical and NA operands.
na <- function(n) if (is.na(n > 0)) "na" else "ok"
for (v in c(NA, 1)) print(na(v + 1))
