# R_execMethod runs S4 methods without applyClosure's exec continuation
# handling (eval.c): a Tailcall() in a method's tail position is an error.
library(methods)
setGeneric("tc", function(x) standardGeneric("tc"))
setMethod("tc", "numeric", function(x) Tailcall(sum, x, 1))
r <- tryCatch(tc(1), error = function(e) e)
print(conditionMessage(r))
# Out of tail position it is an ordinary call.
setMethod("tc", "character", function(x) { y <- Tailcall(paste0, x, "!"); y })
print(tc("a"))
