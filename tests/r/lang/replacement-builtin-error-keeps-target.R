# A replacement function that is an ordinary builtin, failing, leaves
# the target unchanged (#527).
x <- c(a = 1)
`foo<-` <- exp
r <- try(foo(x) <- 2, silent = TRUE)
cat(r)
print(x)
l <- list(a = list(b = 1))
r <- try(foo(l$a$b) <- 2, silent = TRUE)
cat(r)
print(l)
f <- function() { y <- 1:3; r <- try(foo(y) <- 2, silent = TRUE); cat(r); print(y) }
f()
# Arity errors from in-place replacements leave the target too.
x <- c(a = 1)
r <- try(names(x, 2) <- "b", silent = TRUE); cat(r); print(x)
r <- try(class(x, 2) <- "b", silent = TRUE); cat(r); print(x)
r <- try(attr(x) <- "b", silent = TRUE); cat(r); print(x)
r <- try(environment(x, 1, 2) <- NULL, silent = TRUE); cat(r); print(x)
l <- list(a = c(b = 1))
r <- try(names(l$a, 2) <- "z", silent = TRUE); cat(r); print(l)
r <- try(attr(l$a) <- "z", silent = TRUE); cat(r); print(l)
f <- function() { y <- c(a = 1); r <- try(names(y, 2) <- "b", silent = TRUE); cat(r); print(y); r <- try(attr(y, "q", 3) <- 1, silent = TRUE); cat(r); print(y) }
f()
r <- try(x[[1, 2, 3]] <- 5, silent = TRUE); cat(r); print(x)
