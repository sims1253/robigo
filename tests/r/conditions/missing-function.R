# "could not find function" reports the failing call itself
f <- function(x) x
tryCatch(f(nope(1)), error = function(e) conditionCall(e))
g <- function() undefined_fn(2)
tryCatch(g(), error = function(e) conditionCall(e))
x <- 1:3
tryCatch(names(x) <- nofun(x), error = function(e) conditionCall(e))
h <- function(n) if (n > 0) h(n - 1) else missing_helper(n)
h(2)
