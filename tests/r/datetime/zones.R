zones <- c("America/New_York", "Europe/Berlin", "Australia/Sydney", "Asia/Kolkata",
           "America/Sao_Paulo", "Pacific/Chatham", "Europe/London", "Africa/Casablanca",
           "America/St_Johns", "Asia/Tehran", "Pacific/Apia", "Europe/Dublin",
           "Antarctica/Troll", "America/Santiago", "Europe/Moscow", "UTC", "GMT", "Etc/GMT+5",
           "EST5EDT", "JST-9", "<+0330>-3:30", "Foo/Bar", "Europe/Lisbon", "Asia/Kathmandu")
tt <- c()
for (k in 1:150) tt <- c(tt, -5e9 + k * 61e6 + k * k * 1234.5)
tt <- c(tt, 1710054000 + (-3:3) * 1800, 1730613600 + (-3:3) * 1800, 4102444800 + (0:3) * 15e6)
for (z in zones) {
  lt <- .Internal(as.POSIXlt(tt, z))
  u <- unclass(lt)
  cat(z, "\n")
  print(attr(lt, "tzone"))
  k <- 0
  for (comp in u) {
    k <- k + 1
    cat(comp, "\n")
    if (k == 1) sec <- comp
    if (k == 2) min <- comp
    if (k == 3) hour <- comp
    if (k == 4) mday <- comp
    if (k == 5) mon <- comp
    if (k == 6) year <- comp
    if (k == 7) wday <- comp
    if (k == 8) yday <- comp
    if (k == 9) isdst <- comp
  }
  ct <- .Internal(as.POSIXct(lt, z))
  cat(unclass(ct) - tt, "\n")
  f <- .Internal(format.POSIXlt(lt, "%Y-%m-%d %H:%M:%S %Z %z|%c|%G-%V-%u|%s|%k%l%P", TRUE, 0L))
  cat(f, sep = "\n")
  for (dst in c(-1L, 0L, 1L)) {
    l2 <- list(sec = sec, min = min + 7L, hour = hour + 1L, mday = mday + 3L, mon = mon,
               year = year, wday = wday, yday = yday, isdst = isdst * 0L + dst)
    class(l2) <- c("POSIXlt", "POSIXt")
    cat(dst, unclass(.Internal(as.POSIXct(l2, z))) - tt, "\n")
  }
}
