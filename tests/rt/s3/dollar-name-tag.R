# `$` dispatch passes the call's own name cell (fixSubset3Args): a tag on it
# reaches the method
x <- structure(list(a = 1), class = "foo")
`$.foo` <- function(x, ...) { a <- list(...); cat("names:", if (is.null(names(a))) "none" else names(a), "value:", a[[1]], "\n"); invisible() }
x$a
`$`(x, a)
`$`(x, name = a)
`$`(x, "a")
f <- function(x) { x$b; `$`(x, nm = b); invisible() }
invisible(f(x)); invisible(f(x)); invisible(f(x))
