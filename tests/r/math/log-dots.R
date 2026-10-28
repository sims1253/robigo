# log() is a special that evaluates its arguments like evalListKeepMissing:
# `...` is expanded and a missing base keeps its default
f <- function(...) log(...)
f(1)
f(8, 2)
f(x = 8, base = 2)
h <- function(x, b) log(x, b)
h(8)
Funcall <- function(f, ...) f(...)
Reduce(Funcall, list(log, exp, acos, cos), 0, right = TRUE)
