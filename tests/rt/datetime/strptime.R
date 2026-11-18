# datetime.c's do_strptime and Rstrptime.h's R_strptime, in UTC.
# `today`: the elements a format without a date fills from today's date,
# whose date fields are masked (NA) so the recording does not depend on
# the day it is run; `year`: the format gives the year, which is kept
show <- function(lt, today = integer(0), year = FALSE) {
    y <- unclass(lt)
    masked <- c("mday", "mon", if (!year) "year", "wday", "yday")
    for (n in names(y)) {
        v <- y[[n]]
        if (n %in% masked) v[today] <- NA
        cat(n, ": "); print(v)
    }
}
show(strptime(c(a = "2024-02-29 13:45:30.25", b = "1999-12-31 23:59:60"),
              "%Y-%m-%d %H:%M:%OS", tz = "UTC"))
attributes(unclass(strptime("2020-01-01", "%Y-%m-%d", tz = "UTC")))[c("tzone", "balanced")]
class(strptime("2020-01-01", "%Y-%m-%d", tz = "UTC"))
# names and formats recycled
show(strptime(c(p = "01/02/2003"), c("%d/%m/%Y", "%m/%d/%Y"), tz = "GMT"))
# month and day names, any case, full or abbreviated
show(strptime(c("Monday March 4 2024", "sat FEB 3 2001", "Tue jun 30 2020"), "%A %B %d %Y", tz = "UTC"))
show(strptime("3pm 07 Nov 05", "%I%p %d %b %y", tz = "UTC"))
show(strptime(c("12:00:00 AM", "12:30:00 PM", "01:05:00 am"), "%r", tz = "UTC"), today = 1:3)
# day of the year, weeks, weekday numbers, centuries
show(strptime(c("2023 060", "2024 366", "2023 366"), "%Y %j", tz = "UTC"))
show(strptime(c("2024 10 3", "2024 00 0"), "%Y %U %u", tz = "UTC"))
show(strptime("2024 10 1", "%Y %W %w", tz = "UTC"))
show(strptime(c("20 24", "19 69"), "%C %y", tz = "UTC"), today = 1:2, year = TRUE)
show(strptime(c("68", "69"), "%y", tz = "UTC")$year)
# the composite formats
show(strptime(c("2001-02-03", "02/03/04", "10:11", "10:11:12"), c("%F", "%D", "%R", "%T"), tz = "UTC"), today = 3:4)
show(strptime("Sat Feb  3 04:05:06 2001", "%c", tz = "UTC"))
show(strptime(c(" 5  7 2001", "5\t7\n2001"), "%e %k %Y", tz = "UTC"))
show(strptime("100% 2001", "100%% %Y", tz = "UTC"), today = 1, year = TRUE)
# offsets
show(strptime(c("2020-06-01 12:00 +0200", "2020-06-01 12:00 -0930", "2020-06-01 12:00 +0060"),
              "%Y-%m-%d %H:%M %z", tz = "UTC"))
# seconds since the epoch, 24:00:00, and the fractional seconds' forms
show(strptime(c("0", "86399", "1700000000"), "%s", tz = "UTC"))
show(strptime(c("2020-12-31 24:00:00", "2020-02-30 00:00:00"), "%Y-%m-%d %H:%M:%S", tz = "UTC"))
show(strptime(c("1:2:3.75", "1:2:Inf", "1:2:-1", "1:2:99"), "%H:%M:%OS", tz = "UTC"), today = 1:4)
show(strptime("1:2:3.141", "%H:%M:%OS3", tz = "UTC"), today = 1)
# what does not match, and NA
show(strptime(c("2020-13-01", "x", NA, "2020-1-1junk"), "%Y-%m-%d", tz = "UTC"))
show(strptime(character(0), "%Y", tz = "UTC"))
# a missing day of the month with a month is invalid; a year alone takes
# today's month and day
show(strptime("2001 03", "%Y %m", tz = "UTC")$mday)
# %OS reads with wcstod in the UTF-8 locale (its white space), a negative
# week-derived day of the year (C reads before __mon_yday's leap row)
show(strptime("2000-01-01/ 2.5!", "%F/%OS!", tz = "UTC"))
show(strptime(c("2024 00 0", "2023 00 0"), "%Y %U %w", tz = "UTC"))
# day 366 of a common year with a weekday: glibc_fix runs past December
show(strptime(c("2023 366 0", "2024 366 0", "2023 365 0"), "%Y %j %w", tz = "UTC"))
