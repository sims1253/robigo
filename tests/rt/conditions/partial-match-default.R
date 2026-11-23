# A partialMatchWarning as the default handling shows it: deferred, printed
# at the end of the top-level call; under options(warn = 1) at once; under
# warn = 2 the error, against the call.
options(warnPartialMatchArgs = TRUE)
f <- function(abc) abc
f(a = 1)
options(warn = 1)
f(ab = 2)
options(warn = 2)
r <- try(f(a = 3), silent = TRUE); cat(r)
r <- tryCatch(f(a = 4), warning = function(w) class(w)); print(r)
