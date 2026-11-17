# do_strptime's argument checks, the %Z error and R_strptime's warnings.
f <- function(...) tryCatch(.Internal(strptime(...)), error = function(e) conditionMessage(e))
f(1, "%Y", "UTC")
f("2001", 1, "UTC")
f("2001", character(0), "UTC")
f("2001", "%Y", 1)
f("2001", "%Y", c("UTC", "GMT"))
f("10 UTC", "%H %Z", "UTC")
g <- function(x, f) .Internal(strptime(x, f, "UTC"))$year
r <- withCallingHandlers(g("2001 +1500", "%Y %z"),
                         warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
r
r <- withCallingHandlers(.Internal(strptime("2023 366", "%Y %j", "UTC"))$mon,
                         warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
r
r <- withCallingHandlers(.Internal(strptime("2023 54 1", "%Y %U %u", "UTC"))$mon,
                         warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
r
s <- "é"
Encoding(s) <- "bytes"
f(s, "%Y", "UTC")
g("2001 x", "%Y %Z")
