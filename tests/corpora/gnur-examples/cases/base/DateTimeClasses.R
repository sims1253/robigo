### Name: DateTimeClasses
### Title: Date-Time Classes
### Aliases: DateTimeClasses POSIXct POSIXlt POSIXt print.POSIXct
###   print.POSIXlt summary.POSIXct summary.POSIXlt +.POSIXt -.POSIXt
###   Ops.POSIXt Math.POSIXt Summary.POSIXct Math.POSIXlt Summary.POSIXlt
###   $<-.POSIXlt [.POSIXct [<-.POSIXct [[.POSIXct [.POSIXlt [<-.POSIXlt
###   [[.POSIXlt [[<-.POSIXlt as.data.frame.POSIXlt as.list.POSIXct
###   as.list.POSIXlt .leap.seconds anyNA.POSIXlt is.finite.POSIXlt
###   is.infinite.POSIXlt is.na.POSIXlt is.nan.POSIXlt c.POSIXct c.POSIXlt
###   as.matrix.POSIXlt length.POSIXlt length<-.POSIXct length<-.POSIXlt
###   mean.POSIXct mean.POSIXlt str.POSIXt duplicated.POSIXlt
###   unique.POSIXlt split.POSIXct names.POSIXlt names<-.POSIXlt date-time
### Keywords: utilities chron

### ** Examples
## IGNORE_RDIFF_BEGIN
(z <- Sys.time())             # the current date, as class "POSIXct"

Sys.time() - 3600             # an hour ago

as.POSIXlt(Sys.time(), "GMT") # the current time in GMT
format(.leap.seconds)         # the leap seconds in your time zone
## IGNORE_RDIFF_END
print(.leap.seconds, tz = "America/Los_Angeles")  # and in Seattle's

## look at *internal* representation of "POSIXlt" :
leapS <- as.POSIXlt(.leap.seconds)
names(unclass(leapS)) ; is.list(leapS)
## str() on inner structure needs unclass(.):
utils::str(unclass(leapS), vec.len = 7)
## show all (apart from "tzone" attr):
data.frame(unclass(leapS))

## Extracting *single* components of POSIXlt objects:
leapS[1 : 5, "year"]
leapS[17:22, "mon" ]

##  length(.) <- n   now works for "POSIXct" and "POSIXlt" :
for(lpS in list(.leap.seconds, leapS)) {
    ls <- lpS; length(ls) <- 12
    l2 <- lpS; length(l2) <- 5 + length(lpS)
    stopifnot(exprs = {
      ## length(.) <- * is compatible to subsetting/indexing:
      identical(ls, lpS[seq_along(ls)])
      identical(l2, lpS[seq_along(l2)])
      ## has filled with NA's
      is.na(l2[(length(lpS)+1):length(l2)])
    })
}



