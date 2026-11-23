# The partialMatchWarning of argument matching under
# options(warnPartialMatchArgs = TRUE) (errors.c:
# R_makePartialMatchWarningCondition, signalled by R_signalWarningCondition
# as base's warning(cond)): its class, fields and call from a closure's
# matchArgs, an S3 method's, a builtin's, and check1arg's (call()); the
# calls a handler sees.
options(warnPartialMatchArgs = TRUE)
h <- function(w) {
  print(class(w)); print(w$argument); print(w$formal); print(conditionCall(w))
  print(names(unclass(w)))
  print(vapply(sys.calls(), function(x) deparse(x[[1L]])[1L], ""))
  invokeRestart("muffleWarning")
}
f <- function(abc) abc
r <- withCallingHandlers(f(a = 1), warning = h); print(r)
g <- function(x, ...) UseMethod("g"); g.default <- function(x, abc, ...) abc
r <- withCallingHandlers(g(1, ab = 3), warning = h); print(r)
r <- withCallingHandlers(seq_len(length = 2), warning = h); print(r)
r <- withCallingHandlers(call(na = "sum"), warning = h); print(r)
