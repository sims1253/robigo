# seq_along() on an object dispatches length() and converts the result
# with asInteger(), which warns when the conversion introduces an NA.
x <- structure(pi, class = "testit")
length.testit <- function(x) 3.7
seq_along(x)
length.testit <- function(x) 2L
seq_along(x)
length.testit <- function(x) "2"
seq_along(x)
length.testit <- function(x) c(3, 100)
seq_along(x)
length.testit <- function(x) "OK"
r <- tryCatch(seq_along(x), warning = function(w) conditionMessage(w))
r
# asInteger only looks at the first element, so later ones never warn.
op <- options(nwarnings = c("5", "x"))
getOption("nwarnings")
options(op)
