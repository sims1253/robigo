# needs: lubridate
# Date/time parsing and arithmetic through the compiled timechange
# code, pinned to UTC by the harness environment.
suppressPackageStartupMessages(library(lubridate))
d <- ymd_hms(c("2026-03-29 01:30:00", "2024-02-29 23:59:59"), tz = "UTC")
print(d + days(1))
print(floor_date(d, "month"))
print(interval(d[1], d[2]) %/% days(1))
print(ymd("2026-01-31") %m+% months(1))
print(wday(d, label = TRUE, abbr = FALSE))
