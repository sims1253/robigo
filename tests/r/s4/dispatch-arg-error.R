# An error while evaluating a dispatch argument is wrapped once by
# methods' argEvalCleanup handler (R_evalHandleError), not again by the
# same handler when the wrapped error is signalled.
library(methods)
setGeneric("f", function(x) standardGeneric("f"))
setMethod("f", "numeric", function(x) x)
err <- tryCatch(f(stop("this is mentioned")), error = identity)
print(conditionMessage(err)); print(class(err)); print(conditionCall(err))
g <- function(y) f(y)
err <- tryCatch(g(stop("inner")), error = identity)
print(conditionMessage(err))
setGeneric("f2", function(x, y) standardGeneric("f2"))
setMethod("f2", signature("numeric", "numeric"), function(x, y) x + y)
err <- tryCatch(f2(1, stop("second")), error = identity)
print(conditionMessage(err))
# a calling handler outside sees the wrapped error once
n <- 0
r <- withCallingHandlers(tryCatch(f(stop("counted")), error = function(e) "caught"),
                         error = function(e) n <<- n + 1)
print(r); print(n)
res <- try(f(stop("via try")), silent = TRUE)
cat(res)
