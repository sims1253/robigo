# .Internal(strptime(x, format, tz)) outside UTC, printed unclassed
show <- function(x) { print(unclass(x)); invisible() }
sp <- function(x, f, tz) .Internal(strptime(x, f, tz))
show(sp(c(a = "2024-07-01 12:00:00", b = "2024-01-01 00:30:00", c = "1945-06-05 03:00:00", d = NA, e = "nonsense"),
        "%Y-%m-%d %H:%M:%S", "Europe/Berlin"))
# the spring-forward gap and the fall-back overlap
show(sp(c("2024-03-10 02:30:00", "2024-11-03 01:30:00", "2024-03-10 03:30:00"), "%Y-%m-%d %H:%M:%S", "America/New_York"))
# an offset in the input: to time_t and back in the zone
show(sp(c("2024-07-01 12:00:00 +0530", "2024-07-01 12:00:00 -0800"), "%Y-%m-%d %H:%M:%S %z", "Europe/Berlin"))
# fractions, a missing date (the current year/month/day: not shown), %j
x <- sp("2024-07-01 12:34:56.75", "%Y-%m-%d %H:%M:%OS", "Asia/Tokyo"); x$sec; x$zone; x$gmtoff; attr(x, "tzone")
x <- sp(c("2024 060", "2023 366"), "%Y %j", "Europe/Berlin"); x$mon; x$mday
# "" is the TZ variable
Sys.setenv(TZ = "Australia/Adelaide"); x <- sp("2024-01-15 10:00", "%Y-%m-%d %H:%M", ""); x$zone; x$isdst; attr(x, "tzone")
Sys.unsetenv("TZ")
# warnings before the error that stops the loop, in order; warn = 2
f <- function() sp(c("2023 366", "2022 366", "2001 x"), c("%Y %j", "%Y %j", "%Y %Z"), "Europe/Berlin")
withCallingHandlers(tryCatch(f(), error = function(e) conditionMessage(e)),
  warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
op <- options(warn = 2); tryCatch(sp("2023 366", "%Y %j", "Europe/Berlin"), error = function(e) conditionMessage(e)); options(op)
tryCatch(sp("2023 366", "%Y %j", "Europe/Berlin"), warning = function(w) conditionMessage(w))
# the zone is put back
Sys.getenv("TZ", "<unset>")
# through base R
as.POSIXct("2024-07-01 12:00:00", tz = "Europe/Berlin")
format(strptime("2024-07-01 12:00", "%Y-%m-%d %H:%M", tz = "America/New_York"), "%Y-%m-%d %H:%M %Z")
# conditions are signalled in the zone set_tz set: a handler sees TZ, and a
# change it makes holds for the next conversion until reset_tz
Sys.setenv(TZ = "UTC")
x <- withCallingHandlers(
  .Internal(strptime(c("2023 366", "2024-07-01 12:00"), c("%Y %j", "%Y-%m-%d %H:%M"), "Europe/Berlin")),
  warning = function(w) { print(Sys.getenv("TZ")); Sys.setenv(TZ = "Asia/Tokyo"); invokeRestart("muffleWarning") })
print(x$zone); print(Sys.getenv("TZ"))
r <- tryCatch(withCallingHandlers(.Internal(strptime("2001 x", "%Y %Z", "Europe/Berlin")),
  error = function(e) print(Sys.getenv("TZ"))), error = function(e) conditionMessage(e))
r; Sys.getenv("TZ")
z <- .Internal(as.POSIXlt(0, "UTC")); z$year <- 2147483647L
withCallingHandlers(tryCatch(.Internal(format.POSIXlt(z, strrep("%Y", 300), FALSE, 0L)), error = function(e) conditionMessage(e)),
  error = function(e) print(Sys.getenv("TZ")))
Sys.unsetenv("TZ")
