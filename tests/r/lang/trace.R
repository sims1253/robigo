# .primTrace / .primUntrace (debug.c:do_trace): the RTRACE bit and the
# "trace:" reports, from the AST interpreter and from compiled code.
f <- function(x) x + 1
trace(f)
f(1)
g <- function(x) f(x)
g(2)
untrace(f)
g(2)

## builtins, including the VM's no-call fast paths
trace(sum)
sum(1:3)
h <- function(x) sum(x)
h(1:2)
untrace(sum)

trace(length)
k <- function(x) length(x)
k(1:3)
untrace(length)
k(1:3)

trace(is.na)
m <- function(x) is.na(x)
m(c(1, NA))
untrace(is.na)
m(c(1, NA))

## a string names the function to trace
trace("sum")
sum(1:2)
untrace("sum")
sum(1:2)
