# Subscript errors carry GNU R's condition fields and classes
# (errors.c: R_makeOutOfBoundsError, R_makeMissingSubscriptError[1]).
tryCid <- function(expr) tryCatch(expr, error = identity)
show <- function(e) { print(class(e)); str <- unclass(e); print(names(str)); print(str[-1L]) }
# missing subscript: get1index has no object; NULL[[]] and [[<- do
show(tryCid(c(a = 1, 2)[[]]))
show(tryCid(NULL[[]]))
xx <- c(a = 1, 2:3)
show(tryCid(xx[[]] <- pi))
EN <- tryCid(NULL[[]])
nm <- c("call", "object")
stopifnot(nm %in% names(EN), identical(EN[nm], list(call = quote(NULL[[]]), object = NULL)))
# out of bounds: [[ with one and several subscripts
show(tryCid(xx[[5]]))
show(tryCid(xx[["b"]]))
show(tryCid(xx[[-7]]))
m <- matrix(1:4, 2, dimnames = list(c("r1", "r2"), c("c1", "c2")))
show(tryCid(m[[3, 1]]))
show(tryCid(m[[1, 5]]))
# matrix and array subsetting: columns are subscript 0, rows 1
show(tryCid(m[3, 1]))
show(tryCid(m[1, 7]))
show(tryCid(m[3, 7]))
show(tryCid(m[1:2, c(1, 9)]))
show(tryCid(m["r3", 1]))
show(tryCid(m[1, "zz"]))
a <- array(1:8, c(2, 2, 2))
show(tryCid(a[1, 1, 4]))
show(tryCid(a[1, 3, 1]))
# matrix-indexing by a matrix
show(tryCid(m[cbind(1, 5)]))
show(tryCid(m[cbind("r1", "zz")]))
# [[<- out of bounds
l <- list(1, 2)
show(tryCid(m[[3, 1]] <- 9L))
# not subsettable
show(tryCid(sum[[1]]))
show(tryCid(quote(a)[1]))
