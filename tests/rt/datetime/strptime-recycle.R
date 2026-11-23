# do_strptime's elements: formats recycled (each converted once for the
# elements it serves), NA elements between them, zone names shared by rows,
# and the order of an invalid input's error against an invalid format's.
p <- function(lt) print(unclass(lt)[c("year", "mon", "mday", "hour", "isdst", "zone", "gmtoff")])
x <- c("2024-01-15 10:00", NA, "2024-07-15 10:00", "15/07/2024", "2024-01-15 10:00", "bad")
f <- c("%Y-%m-%d %H:%M", "%d/%m/%Y")
p(.Internal(strptime(x, f, "Europe/Berlin")))
p(.Internal(strptime(x, "%Y-%m-%d %H:%M", "UTC")))
p(.Internal(strptime(c("2024-01-15", "2024-07-15"), c("%Y-%m-%d", "%Y-%m-%d", "%Y-%m-%d"), "America/New_York")))
p(.Internal(strptime(NA_character_, "%Y", "UTC")))
e <- function(...) tryCatch(.Internal(strptime(...)), error = function(e) conditionMessage(e))
bad <- "\xff"
e(bad, "%Y", "UTC")
e("2001", bad, "UTC")
e(bad, bad, "UTC")
e(c("2001", bad), c("%Y", "%Y"), "UTC")
e(c("2001", "2002"), c("%Y", bad), "UTC")
e(strrep("1", 1001), "%Y", "UTC")
e("2001", strrep("%Y", 501), "UTC")
