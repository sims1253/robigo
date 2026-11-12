# x[i] <- <S4 object>: SubassignTypeFix calls the value's as.vector()
# method (dispatch_asvector), then fixes types again.
library(methods)
setClass("A", representation(stuff = "numeric"))
as.vector.A <- function(x, mode = "any") x@stuff
v <- c(3.5, 0.1)
a <- new("A", stuff = v)
x <- y <- numeric(10)
x[3:4] <- a
y[3:4] <- v
stopifnot(identical(x, y))
i <- 1:2; i[2:3] <- a; print(i)
ch <- c("p", "q"); ch[1:2] <- a; print(ch)
z <- list(1, 2); z[2] <- list(a); print(z[[2]]@stuff)
# MatrixAssign, ArrayAssign and [[<- read XLENGTH/LENGTH of the S4 cell,
# which is undefined in GNU R (its output varies run to run), so they are
# not tested here.
# a method returning another S4 object is dispatched on again
setClass("B", representation(inner = "A"))
as.vector.B <- function(x, mode = "any") x@inner
x[5:6] <- new("B", inner = a); print(x)
# no method: DispatchOrEval leaves the argument pairlist behind
setClass("C", representation(stuff = "numeric"))
tryCatch(x[1] <- new("C", stuff = 1), error = function(e) print(conditionMessage(e)))
# an S4 as.vector method
setClass("D", representation(stuff = "numeric"))
setMethod("as.vector", "D", function(x, mode = "any") rev(x@stuff))
q <- numeric(3); q[2:3] <- new("D", stuff = c(1, 2)); print(q)
# the method is found from the assignment's environment
f <- function() {
  as.vector.A <- function(x, mode) -x@stuff
  q <- numeric(2); q[] <- a; q
}
print(f())
# the method sees the assignment target still bound
setClass("E", representation(stuff = "numeric"))
as.vector.E <- function(x, mode = "any") length(target)
target <- c(10, 20); target[1] <- new("E"); print(target)
g <- function() { target <- c(10, 20, 30); target[2] <- new("E"); target }
print(g())
