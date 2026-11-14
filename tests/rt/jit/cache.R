# GNU R's JIT cache (R_cmpfun, eval.c): compiling a closure reuses the code
# of an identical body compiled for a compatible environment, with the
# decisions it was compiled with; an identical body in a frame binding a
# name the entry lacks is marked NOJIT.
bt <- function(a) .Internal(getBindingType(quote(a), environment()))
t1 <- function() { for (i in 1) NULL; bt(1) }
print(t1())
mk <- function() { z <- 1; if (FALSE) browser(); function() { for (i in 1) NULL; bt(1) } }
t2 <- mk()
print(t2()); print(t2()); print(t2())
f <- function(x) x * 2
print(f(1)); print(f(1))
`*` <- function(e1, e2) "user times"
g <- function(x) x * 2
print(g(1)); print(g(1)); print(g(1))
h <- function(y) y * 2
print(h(1)); print(h(1))
