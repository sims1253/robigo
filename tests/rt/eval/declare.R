# eval.c's declare: a special that evaluates nothing and returns NULL
# invisibly.
declare()
x <- declare(stop("not evaluated"), y = undefined_variable)
x
withVisible(declare(1))
f <- function() { declare(type(x = integer(1))); x <- 1L; x }
f()
typeof(declare)
is.primitive(declare)
g <- function(...) declare(...)
g(stop("no"))
(declare())
