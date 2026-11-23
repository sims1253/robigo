# _R_CHECK_BROWSER_NONINTERACTIVE_ makes browser() an error when not
# interactive; browser(expr = FALSE) still does nothing.
Sys.setenv("_R_CHECK_BROWSER_NONINTERACTIVE_" = "true")
r <- tryCatch(browser(), error = function(e) conditionMessage(e))
r
browser(expr = FALSE)
f <- function() browser()
tryCatch(f(), error = function(e) conditionCall(e))
Sys.setenv("_R_CHECK_BROWSER_NONINTERACTIVE_" = "yes")
browser()
c
Sys.unsetenv("_R_CHECK_BROWSER_NONINTERACTIVE_")
# deparse.max.lines limits the call printed
op <- options(deparse.max.lines = 3)
f <- function(...) browser()
do.call(f, list(1:3, a = letters, b = LETTERS, c = seq(0.5, 20, by = 0.5)))
c
options(op)
# debug() errors and warnings
tryCatch(debug(1), error = function(e) conditionMessage(e))
undebug(function() 1)
isdebugged(sum)
debug("sum")
isdebugged("sum")
undebug(sum)
tryCatch(browserText(), error = function(e) conditionMessage(e))
tryCatch(browserText(0), error = function(e) conditionMessage(e))
