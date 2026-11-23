# forceAndCall() and calls of builtins held in variables, in compiled code
Reduce(`+`, 1:5, accumulate = TRUE)
Reduce(function(a, b) paste(a, b), letters[1:4], accumulate = TRUE, right = TRUE)
Reduce(`+`, list(), 0)
f <- function(a, b) { cat("in f\n"); a * b }
side <- function(tag, v) { cat("force", tag, "\n"); v }
g <- function() forceAndCall(1, f, side("a", 2), side("b", 3))
g()
h <- function(fun) forceAndCall(2, fun, side("x", 1), side("y", 2))
h(function(p, q) { cat("body\n"); p + q })
h(`+`)
h(function(p, q) { cat("lazy body\n"); 0 })
k <- function(fun) forceAndCall(1, fun, side("only", 5))
k(function(z) { cat("never forced\n"); 1 })
tryCatch((function() forceAndCall(1, nonexistent_fn, 1))(), error = function(e) print(conditionMessage(e)))
w <- function(fn) fn(side("m", 1), side("n", 2))
w(max)
v <- function() { fn <- sum; x <- 1; fn(x <- 10, x) }
v()
sc <- function(fn) sys.call()
(function(fun) forceAndCall(1, fun, 7))(function(z) sys.call())
Map(function(a, b) a + b, 1:3, 4:6)
Filter(function(v) v > 2, 1:5)
