step <- function(x, ...) UseMethod("step")
step.a <- function(x, ...) { cat("a; .Class:", .Class, "\n"); NextMethod() }
step.b <- function(x, ...) { cat("b; .Class:", .Class, "\n"); NextMethod() }
step.default <- function(x, ...) { cat("default\n"); invisible(x) }
x <- structure(1, class = c("a", "b"))
step(x)
y <- step(x)
print(unclass(y))
add <- function(x, n) UseMethod("add")
add.big <- function(x, n) NextMethod(n = n * 10)
add.default <- function(x, n) n
add(structure(1, class = "big"), 2)
