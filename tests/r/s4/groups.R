# S4 group generics (Compare, Summary) on a class extending list; comparisons with lists (relop.c); as.numeric/as.double methods
library(methods)
setClass("L", contains = "list")
setMethod("Compare", signature(e1="L", e2="ANY"), function(e1,e2) sapply(e1, .Generic, e2=e2))
setMethod("Summary", "L", function(x, ..., na.rm=FALSE) {x <- unlist(x); callNextMethod()})
setMethod("[", signature(x="L", i="ANY", j="missing",drop="missing"), function(x,i,j,drop) new(class(x), x@.Data[i]))
setMethod("xtfrm", "L", function(x) xtfrm(unlist(x@.Data)))
mean.L <- function(x, ...) new("L", mean(unlist(x@.Data), ...))
x <- new("L", 1:3); x2 <- x[-2]
print(unlist(x2))
mx <- stats::median(x); print(is(mx, "L")); print(mx == 2)
print(max(x))
print(stats::median(x2) == x[2])
2 == list(2)
list(1, "a") == "a"
list(1L, 2L) < 3L
list(TRUE) == TRUE
r <- tryCatch(list(1) == list(1), error = function(e) conditionMessage(e)); r
r <- tryCatch(list(1:2) == 1, error = function(e) conditionMessage(e)); r
r <- tryCatch(expression(a) == 1, error = function(e) conditionMessage(e)); r
x <- list(a = 1, b = 2); x == 2
list(1+1i) == 1+1i
setClass("c1", representation(x = "numeric")); x_c1 <- new("c1", x = 1)
setMethod("as.numeric", "c1", function(x, ...) 42 + pi)
identical(as.numeric(x_c1), as.double(x_c1))
