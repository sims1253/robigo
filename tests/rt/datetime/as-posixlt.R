# .Internal(as.POSIXlt(x, tz)): UTC and other zones, printed unclassed
show <- function(x) { print(unclass(x)); invisible() }
x <- c(a = 0, b = 1e9 + 0.25, c = -1.5, d = NA, e = Inf, f = 86399.999)
show(.Internal(as.POSIXlt(x, "UTC")))
show(.Internal(as.POSIXlt(c(1.7e9, -2e9), "GMT")))
# a zone with daylight saving: summer, winter, the gap and a historical name
show(.Internal(as.POSIXlt(c(1719835200, 1704067200, -775526400, -2e9), "Europe/Berlin")))
show(.Internal(as.POSIXlt(c(1710054000, 1710057600, 4102444800), "America/New_York")))
# "" is the TZ variable, when it is set
Sys.setenv(TZ = "Asia/Tokyo"); show(.Internal(as.POSIXlt(0, ""))); Sys.getenv("TZ")
Sys.setenv(TZ = "UTC"); attributes(.Internal(as.POSIXlt(0, "")))
Sys.unsetenv("TZ"); attr(.Internal(as.POSIXlt(0, "Australia/Adelaide")), "tzone"); Sys.getenv("TZ", "<unset>")
# coercion of x, and the errors
.Internal(as.POSIXlt(TRUE, "UTC"))$sec
tryCatch(.Internal(as.POSIXlt("x", "UTC")), warning = function(w) conditionMessage(w))
tryCatch(.Internal(as.POSIXlt(0, c("UTC", "GMT"))), error = function(e) conditionMessage(e))
tryCatch(.Internal(as.POSIXlt(0, 1)), error = function(e) conditionMessage(e))
tryCatch(.Internal(as.POSIXlt(0)), error = function(e) conditionMessage(e))
length(.Internal(as.POSIXlt(numeric(0), "UTC"))$year)
# through base R
as.POSIXlt(.POSIXct(1e9, tz = "UTC"))$hour
unlist(unclass(as.POSIXlt(.POSIXct(1e9), tz = "Europe/London"))[1:9])
# as.POSIXct outside UTC
unclass(.Internal(as.POSIXct(.Internal(as.POSIXlt(c(1719835200, -775526400), "Europe/Berlin")), "Europe/Berlin")))
lt <- .Internal(as.POSIXlt(1710054000 + 3600, "America/New_York")); lt$isdst <- -1L
unclass(.Internal(as.POSIXct(lt, "America/New_York")))
# Date2POSIXlt and POSIXlt2Date
show(.Internal(Date2POSIXlt(c(x = 0, y = 19000.75, z = -719528, w = NA, v = 1e12), "")))
attr(.Internal(Date2POSIXlt(1, "Europe/Paris")), "tzone")
d <- .Internal(POSIXlt2Date(.Internal(as.POSIXlt(c(p = 1e9, q = -1e9, r = NA), "UTC")))); unclass(d)
lt <- .Internal(as.POSIXlt(0, "UTC")); lt$mday <- 40L; lt$hour <- 30L; unclass(.Internal(POSIXlt2Date(lt)))
tryCatch(.Internal(POSIXlt2Date(list(1))), error = function(e) conditionMessage(e))
tryCatch(.Internal(Date2POSIXlt(1, NULL)), error = function(e) conditionMessage(e))
# set_tz's error comes before as.POSIXct's component checks
x <- .Internal(as.POSIXlt(0, "UTC")); x$sec <- numeric()
Sys.setenv(TZ = strrep("x", 1001))
tryCatch(.Internal(as.POSIXct(x, "Europe/Berlin")), error = function(e) conditionMessage(e))
tryCatch(.Internal(as.POSIXlt(0, "Europe/Berlin")), error = function(e) conditionMessage(e))
tryCatch(.Internal(as.POSIXct(x, "UTC")), error = function(e) conditionMessage(e))
Sys.unsetenv("TZ")
# names go through setAttrib: short ones are padded
x <- .Internal(as.POSIXlt(0, "UTC")); x$sec <- c(0, 1); names(x$year) <- "a"
names(.Internal(POSIXlt2Date(x))); names(.Internal(as.POSIXct(x, "UTC"))); attributes(.Internal(POSIXlt2Date(x)))
# valid_POSIXlt's LENGTH(x) of what is not a vector
tryCatch(.Internal(POSIXlt2Date(quote(x))), error = function(e) conditionMessage(e))
tryCatch(.Internal(as.POSIXct(quote(f(x)), "UTC")), error = function(e) conditionMessage(e))
tryCatch(.Internal(POSIXlt2Date(y ~ x)), error = function(e) conditionMessage(e))
tryCatch(.Internal(POSIXlt2Date(NULL)), error = function(e) conditionMessage(e))
