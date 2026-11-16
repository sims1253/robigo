# The link-walk family (link-walk.R): a builtin's earlier results while it
# calls FUN again. lapply stores each result in its answer as FUN returns
# it (SET_VECTOR_ELT's link), so when a later call writes `z`, which an
# earlier one returned, the write copies and the answer keeps 1 2. vapply
# copies each value's data into its answer at once: the same. Both through
# the .Internal and through the closures.
x <- c(1, 2)
f <- function(e) { if (e == 2) z[1] <<- 99; z }
g <- function(...) .Internal(lapply(x, f))
z <- c(1, 2)
r <- g()
print(r[[1]])
print(r[[2]])
z <- c(1, 2)
print(lapply(x, f)[[1]])
h <- function(...) .Internal(vapply(x, f, c(0, 0), FALSE))
z <- c(1, 2)
print(c(h()))
z <- c(1, 2)
print(c(vapply(x, f, c(0, 0))))
k <- function(e) { if (e == 2) z[[1]][1] <<- 99; z }
z <- list(c(1, 2))
print(vapply(x, k, list(0))[[1]])
