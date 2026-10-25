# Internal generic dispatch of the vector builtins
x <- 1:3
class(x) <- "foo"
Summary.foo <- function(..., na.rm) {
    cat("Summary.foo:", .Generic, "na.rm =", na.rm, "\n")
    print(sys.call())
    42
}
max(x)
min(x, 10, na.rm = TRUE)
sum(x)
prod(x)
range(x)
all(x)
any(x)
max(5, x)
max.foo <- function(..., na.rm = FALSE) "max.foo called"
max(x)
min(x)
Math.foo <- function(x, ...) {
    cat("Math.foo:", .Generic, "\n")
    unclass(x) * 100
}
cumsum(x)
cummax(x)
rep.foo <- function(x, ...) {
    cat("rep.foo\n")
    print(sys.call())
    "rep result"
}
rep(x, 2)
rep(x, times = 3, each = 2)
rep.int(x, 2)
rep_len(x, 5)
seq.foo <- function(...) "seq.foo"
seq.int(x, 10)
xtfrm.foo <- function(x) c(9, 8, 7)
xtfrm(x)
anyNA.foo <- function(x, recursive = FALSE) "anyNA.foo"
anyNA(x)
length.foo <- function(x) 7L
seq_along(x)
is.unsorted.foo <- function(x, na.rm = FALSE, strictly = FALSE) "is.unsorted.foo"
.Internal(is.unsorted(x, FALSE, FALSE))
y <- 1:3
class(y) <- "bar"
max(y)
cumsum(y)
rep(y, 2)
