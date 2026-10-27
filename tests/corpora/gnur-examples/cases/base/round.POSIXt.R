### Name: round.POSIXt
### Title: Round / Truncate Date-Time Objects
### Aliases: round.POSIXt trunc.POSIXt round.Date trunc.Date
### Keywords: chron

### ** Examples

round(.leap.seconds + 1000, "hour")
## IGNORE_RDIFF_BEGIN
         trunc(Sys.time(), "day")
(timM <- trunc(Sys.time() -> St, "months")) # shows timezone
(datM <- trunc(Sys.Date() -> Sd, "months"))
(timY <- trunc(St, "years")) # + timezone
(datY <- trunc(Sd, "years"))
## IGNORE_RDIFF_END
stopifnot(inherits(datM, "Date"), inherits(timM, "POSIXt"),
          substring(format(datM), 9,10) == "01", # first of month
          substring(format(datY), 6,10) == "01-01", # Jan 1
          identical(format(datM), format(timM)),
          identical(format(datY), format(timY)))



