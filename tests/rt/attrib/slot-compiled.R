# `@<-` from compiled code (cmp.R's setter handler: the name a string) and
# from the AST (do_attrgets puts the string into the call): the call a
# method sees, and the error calls.
x <- structure(list(), class = "foo")
`$<-.foo` <- function(x, name, value) { print(sys.call()); x }
`[<-.foo` <- function(x, i, value) { print(sys.call()); x }
`@<-.foo` <- function(object, name, value) { print(sys.call()); object }
h <- function() { for (i in 1) NULL; x$slt <- 3; x[1] <- 4; x@s <- 5; invisible() }
h()
x$slt <- 3; x[1] <- 4; x@s <- 5
f <- function() { for (i in 1) NULL; z <- 1; z@a <- 2 }
r <- tryCatch(f(), error = function(e) e); print(r); print(conditionCall(r))
r <- tryCatch(f(), error = function(e) e); print(r); print(conditionCall(r))
g <- function() { for (i in 1) NULL; z <- 1; z@"a" <- 2 }
r <- tryCatch(g(), error = function(e) e); print(conditionCall(r))
