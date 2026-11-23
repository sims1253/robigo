library(methods)
setClass("A", representation(x = "numeric"))
setClass("B", contains = "A")
print.A <- function(x, ...) { cat("print.A\n"); invisible(x) }
summary.A <- function(object, ...) "summary.A"
summary.B <- function(object, ...) c("summary.B", NextMethod())
b <- new("B", x = 1)
summary(b)
print(b)
inherits(b, "A")
inherits(b, c("X", "A"), which = TRUE)
f <- function(x) UseMethod("f"); f.A <- function(x) "f.A"
f(b)
r <- tryCatch(f(1), error = function(e) conditionMessage(e)); r
