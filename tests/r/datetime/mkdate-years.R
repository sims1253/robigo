# Days since the epoch of a broken-down date (mkdate00) across the
# 400-year cycles, through as.Date, as.POSIXct and ISOdate, for years far
# before and after 1970.
years <- c(-1e6, -100001, -401, -400, -399, -1, 0, 1, 399, 400, 1600, 1700,
           1900, 1969, 1970, 1971, 2000, 2023, 2100, 9999, 10000, 123456, 1e6)
for (y in years) {
  for (md in list(c(1, 1), c(2, 28), c(3, 1), c(12, 31))) {
    x <- as.POSIXlt("2000-01-01", tz = "UTC")
    x$year <- y - 1900
    x$mon <- md[1] - 1
    x$mday <- md[2]
    d <- unclass(as.Date(x))
    p <- unclass(as.POSIXct(x))
    b <- as.POSIXlt(as.POSIXct(x))
    cat(sprintf("%8d-%02d-%02d %14.0f %18.0f wday=%d yday=%d\n", y, md[1], md[2],
                d, p, b$wday, b$yday))
  }
}
# ISOdate goes through mktime00 with a time of day
unclass(ISOdate(c(-401, 0, 1600, 1969, 1970, 2023, 9999), 7, 15))
unclass(ISOdatetime(1e6, 2, 29, 12, 30, 15, tz = "UTC"))
unclass(ISOdate(-1e6, 3, 1))
# out-of-range fields are normalised first
x <- as.POSIXlt("2023-01-01", tz = "UTC")
x$mon <- 25L
x$mday <- -40L
unclass(as.Date(x))
d <- as.Date(c("0000-02-29", "0001-01-01", "1600-12-31", "9999-12-31"), "%Y-%m-%d")
unclass(d)
format(d, "%u %j")
format(d + 1)
