# `$` and `$<-` dispatch from byte code: the method's call is a copy of the
# code's call with the member as a string (eval.c DOLLAR's duplicate and
# "hack to avoid evaluating the symbol"), and the code's own call is left
# as it was, including when the object is itself a call (g()$b)
`$.foo` <- function(x, name) list(call = sys.call(), name = name)
`$<-.foo` <- function(x, name, value) {
    attr(x, "set") <- list(call = sys.call(), name = name, value = value)
    x
}
x <- structure(list(a = 1), class = "foo")
g <- function() x
f <- function() list(x$a, g()$b)
for (k in 1:3) r <- f()
print(r)
print(body(f))
h <- function() { y <- x; y$z <- 3; attr(y, "set") }
for (k in 1:3) s <- h()
print(s)
print(body(h))
# a failed dispatch on an object without a `$` method: the default
cntxt <- structure(list(toplevel = TRUE, optimize = 2L), class = "compiler_context")
q <- function(n) { for (i in 1:n) v <- cntxt$optimize; v }
print(q(3))
print(body(q))
