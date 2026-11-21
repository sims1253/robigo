# A special primitive reached as an S3 method (objects.c's applyMethod):
# its arguments are the method's promises, and the call its messages name
# is the method's newcall, whose arguments are the expressions.
f <- function(...) tryCatch(..., error = function(e) list(conditionMessage(e), conditionCall(e)))
a1 <- factor(c("a", "b"))
print(f(a1[, ]))
d <- structure(c(18262, 18263), class = "Date")
print(f(d[, ]))
x <- structure(1:2, class = "foo")
`[.foo` <- function(x, ...) NextMethod()
print(f(x[, ]))
print(f(x[5, 6]))
i <- 3; j <- 4
print(f(x[i, j]))
`[[.foo` <- function(x, ...) NextMethod()
print(f(x[[5]]))
print(f(x[[i, j]]))
l <- structure(list(a = 1), class = "bar")
`$<-.bar` <- function(x, name, value) NextMethod()
l$b <- 2; print(unclass(l))
`[<-.bar` <- function(x, i, value) NextMethod()
print(f(`[<-`(l, 1, 2, 3)))
l[1] <- 9; print(unclass(l))
`[[<-.bar` <- function(x, i, value) NextMethod()
print(f(`[[<-`(l, 5, 6, 7)))
# what the method itself sees
`[.qux` <- function(x, ...) { print(sys.call()); print(match.call()); NextMethod() }
q <- structure(1:3, class = "qux")
print(q[2])
print(f(q[, ]))
`$.qux` <- function(x, name) { print(sys.call()); "dollar" }
print(q$abc)
# a replacement special's own error, reached through the method
`[<-.baz` <- function(x, ..., value) NextMethod()
`[[<-.baz` <- function(x, ..., value) NextMethod()
m <- structure(matrix(1:4, 2), class = "baz")
print(f(m[3, 3] <- 1L))
print(f(m[[3, 3]] <- 1L))
print(f(m[1, 2, 3] <- 1L))
k <- 3
print(f(m[[k, k]] <- 1L))
# a special method's own errors name the method's call, its arguments the
# expressions: $'s name check, substitute's argument matching
g <- function(x, name) UseMethod("g"); g.foo <- .Primitive("$")
xf <- structure(list(a = 1), class = "foo"); k <- 1
print(f(g(xf, k)))
h <- function(x, ...) UseMethod("h"); h.foo <- .Primitive("substitute")
xs <- structure(1, class = "foo"); k2 <- 2
print(f(h(xs, expr = k2, expr = k2)))
