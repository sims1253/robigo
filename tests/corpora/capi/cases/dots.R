dyn.load("dots.so")

f <- function(...) environment()
e <- f(stop("never forced"), 2)
l <- as.list.environment(e, all.names = TRUE)
print(typeof(l[[1]]))
# a native call that only reads the list leaves its `...` element alone
print(.Call("capi_dots_names", l))
print(typeof(l[[1]]))
print(length(l[[1]]))
# `...` C finds or returns is `...`
d <- .Call("capi_dots_get", e)
print(typeof(d))
print(length(d))
print(typeof(.Call("capi_dots_elt", l)))
print(.Call("capi_dots_type", l[[1]]))
# and its promises are still unforced
g <- function(...) ..2
print(do.call(g, list(quote(stop("never forced")), 3)))
print(tryCatch(identical(l, as.list.environment(e, all.names = TRUE)), error = conditionMessage))
