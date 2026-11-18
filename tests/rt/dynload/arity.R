# .Internal(dyn.load(...)) and .Internal(dyn.unload(...)) check their
# argument count first (Rdynload.c's checkArity): no argument used to panic
# robigo-rt, and too many gave another error.
show <- function(expr) {
    e <- tryCatch(expr, error = function(e) e)
    cat(conditionMessage(e), "|", deparse(conditionCall(e)), "\n")
}
f <- function() .Internal(dyn.load())
show(f())
g <- function() .Internal(dyn.load("a", TRUE, TRUE, "", 5))
show(g())
h <- function() .Internal(dyn.unload())
show(h())
k <- function() .Internal(dyn.unload("a", "b"))
show(k())
