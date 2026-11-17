# Subscript errors are classed conditions with GNU R's fields
# (errors.c: R_makeOutOfBoundsError, R_makeMissingSubscriptError[1],
# R_makeNotSubsettableError). The object is compared, not printed.
show <- function(e, obj) {
    cat(class(e), "|", conditionMessage(e), "|", deparse(conditionCall(e)), "\n")
    print(names(unclass(e)))
    if (!is.null(e$subscript)) print(e$subscript)
    if (!is.null(e$index)) print(e$index)
    if (!missing(obj)) print(identical(e$object, obj))
}
tc <- function(expr) tryCatch(expr, error = identity)
xx <- c(a = 1, 2:3)
# missing subscript: get1index has no object; NULL[[]] and [[<- do
show(tc(c(a = 1, 2)[[]]))
show(tc(NULL[[]]), NULL)
show(tc(xx[[]] <- pi), xx)
# out of bounds
show(tc(xx[[5]]), xx)
show(tc(xx[["b"]]), xx)
show(tc(xx[[-7]]))
m <- matrix(1:4, 2, dimnames = list(c("r1", "r2"), c("c1", "c2")))
show(tc(m[[3, 1]]), m)
show(tc(m[[1, 5]]), m)
show(tc(m[3, 1]), m)
show(tc(m[1, 7]), m)
show(tc(m[[3, 1]] <- 9L), m)
# not subsettable
show(tc(sum[[1]]), sum)
show(tc(quote(a)[1]), quote(a))
f <- function() 1
show(tc(f$a <- 1), f)
# by class
print(tryCatch(xx[[9]], subscriptOutOfBoundsError = function(e) "out of bounds"))
print(tryCatch(xx[[]], MissingSubscriptError = function(e) "missing"))
print(tryCatch(sum$x, notSubsettableError = function(e) "not subsettable"))
xx[[9]]
