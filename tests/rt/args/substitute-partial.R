# substitute()'s partial matches warn under warnPartialMatchArgs, before
# a matching error.
options(warnPartialMatchArgs = TRUE)
substitute(ex = a + b, list(a = 1))
r <- withCallingHandlers(
  tryCatch(substitute(ex = a, en = list(), c), error = function(e) conditionMessage(e)),
  warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })
r
