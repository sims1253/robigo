# log() is a special: its arguments are evaluated as evalListKeepMissing
# does (`...` expanded), and Math methods see only the arguments given.
f <- function(...) log(...)
print(f(8, 2))
print(f(8))
print(f(x = 100, base = 10))
g <- function(f, ...) f(...)
print(g(log, 81, 3))
print(Reduce(function(f, ...) f(...), list(log, exp, acos, cos), 0, right = TRUE))
X <- list(list(a = pi, b = list(c = 1:1)), d = "a test")
print(rapply(X, log, classes = "numeric", how = "replace", base = 2))
print(sapply(c(1, 10, 100), log, base = 10))
x <- -7
class(x) <- "foo"
Math.foo <- function(x, ...) { cat("Math.foo:", .Generic, "nargs", nargs(), "\n"); -unclass(x) }
print(log(x))
print(log(x, 2))
print(log(x = x))
h <- function(x, ...) log(x, ...)
print(h(x))
print(h(1000, 10))
print(tryCatch(log(), error = conditionMessage))
print(tryCatch(log(y = 2), error = conditionMessage))
print(tryCatch(f(), error = conditionMessage))
