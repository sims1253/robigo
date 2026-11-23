# The arithmetic of zones.R, independent of the system's tzdata: the time
# points built in a loop (exact bits) and the POSIXlt component arithmetic,
# in UTC. (zones.R itself depends on tzdata: Europe/Dublin's 1920s rows
# changed between Debian's tzdata 2026c and 2026e, for GNU R and robigo
# alike.)
tt <- c()
for (k in 1:150) tt <- c(tt, -5e9 + k * 61e6 + k * k * 1234.5)
print(sprintf("%a", tt[c(1, 55:62, 150)]))
print(tt[59], digits = 17)
lt <- .Internal(as.POSIXlt(tt[55:62], "UTC"))
u <- unclass(lt)
for (comp in u[1:9]) cat(comp, "\n")
f <- function(h, d) h + d
s <- 0L
for (k in seq_along(u$hour)) s <- s + f(u$hour[k], 1L)
print(s)
l2 <- list(sec = u$sec, min = u$min + 7L, hour = u$hour + 1L, mday = u$mday + 3L, mon = u$mon,
           year = u$year, wday = u$wday, yday = u$yday, isdst = u$isdst * 0L - 1L)
class(l2) <- c("POSIXlt", "POSIXt")
cat(unclass(.Internal(as.POSIXct(l2, "UTC"))) - tt[55:62], "\n")
