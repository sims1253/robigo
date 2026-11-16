# The link-walk family (link-walk.R): rep, a special, evaluates its
# arguments as DispatchOrEval does: the first holds a link while
# evalListKeepMissing evaluates the rest (each linked until all are). So
# f's `x[1] <<- 99` copies and rep repeats the old x.
x <- c(1, 2)
f <- function() { x[1] <<- 99; 2 }
print(rep(x, f()))
print(x)
g <- function() { y <- c(1, 2); h <- function() { y[1] <<- 99; 2 }; rep(y, h()) }
print(g())
print(g())
print(g())
x <- c(1, 2)
print(rep(x, times = f()))
k <- function() { y <- c(1, 2); h <- function() { y[1] <<- 99; 1 }; rep(y, 2, each = h()) }
print(k())
print(k())
# the object DispatchOrEval finds: the first argument, `...`'s first
# element, or NULL when there is none
print(rep(x = 1:2, 2))
h <- function(...) rep(...)
print(h(1:2, 3))
print(h(x = 1:2, each = 2))
print(h())
print(rep())
h2 <- function(...) rep(..., 2)
print(h2(1:2))
print(h2())
