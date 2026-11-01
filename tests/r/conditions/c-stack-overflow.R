# R: errors.c:R_SignalCStackOverflow, R_makeCStackOverflowError. lapply
# recurses through C, where a compiled closure calling itself would run
# into the node stack instead.
options(expressions = 500000L)
g <- function(n) lapply(1, g)
r <- tryCatch(withCallingHandlers(g(1), error = function(e) cat("calling handlers are skipped\n")),
              error = function(e) e)
print(class(r))
print(names(r))
print(is.numeric(r$usage))
print(conditionCall(r))
print(sub("[0-9]+", "N", conditionMessage(r)))
print(tryCatch(g(1), CStackOverflowError = function(e) "c"))
print(tryCatch(g(1), stackOverflowError = function(e) "so"))
