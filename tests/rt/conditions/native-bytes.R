# Messages are native-encoded bytes, as GNU R's char * messages: an R
# string that is not valid UTF-8 keeps its bytes through stop() and
# warning(), their conditions and their reports.
x <- "a\xffb"
print(identical(tryCatch(stop(x), error = conditionMessage), x))
print(identical(tryCatch(warning(x), warning = conditionMessage), x))
print(identical(tryCatch(warning(x), condition = conditionMessage), x))
r <- withCallingHandlers(warning(x), warning = function(w) {
    print(identical(conditionMessage(w), x))
    invokeRestart("muffleWarning")
})
f <- function() warning(x)
f()
warning(x)
stop(x)
