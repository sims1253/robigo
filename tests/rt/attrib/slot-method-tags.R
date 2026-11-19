# `@<-` and `@` dispatch with the arguments' tags kept (DispatchOrEval,
# fixSubset3Args' shallow_duplicate): a method with `...` before `value`.
x <- structure(list(), class = "foo")
`@<-.foo` <- function(object, name, ..., value) {
  cat("name", name, "value", value, "dots", length(list(...)), "\n")
  object
}
x@s <- 42
h <- function() { for (i in 1) NULL; x@s <- 44; invisible() }
h()
h()
x <- `@<-`(x, "s", value = 46)
r <- tryCatch(`@<-`(x, value = 47, "t"), error = function(e) e); conditionMessage(r); conditionCall(r)
`@.foo` <- function(object, name, ...) cat("name", name, "dots", length(list(...)), "\n")
x@a
`@`(x, name = "b")
`@`(x, "c")
