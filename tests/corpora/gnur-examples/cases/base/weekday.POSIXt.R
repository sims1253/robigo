RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: weekdays
### Title: Extract Parts of a POSIXt or Date Object
### Aliases: weekdays weekdays.POSIXt weekdays.Date months months.POSIXt
###   months.Date quarters quarters.POSIXt quarters.Date julian
###   julian.POSIXt julian.Date
### Keywords: chron

### ** Examples

## No test: 
##D ## first two are locale dependent:
##D weekdays(.leap.seconds)
##D months  (.leap.seconds)
## End(No test)
quarters(.leap.seconds)

## Show how easily you get month, day, year, day (of {month, week, yr}), ... :
## (remember to count from 0 (!): mon = 0..11, wday = 0..6,  etc !!)

##' Transform (Time-)Date vector  to  convenient data frame :
dt2df <- function(dt, dName = deparse(substitute(dt))) {
    DF <- as.data.frame(unclass(as.POSIXlt( dt )))
    `names<-`(cbind(dt, DF, deparse.level=0L), c(dName, names(DF)))
}
## e.g.,
dt2df(.leap.seconds)    # date+time
## No test: 
##D dt2df(Sys.Date() + 0:9) # date
## End(No test)
##' Even simpler:  Date -> Matrix - dropping time info {sec,min,hour, isdst}
d2mat <- function(x) simplify2array(unclass(as.POSIXlt(x))[4:7])
## e.g.,
d2mat(seq(as.Date("2000-02-02"), by=1, length.out=30)) # has R 1.0.0's release date

## No test: 
##D ## Julian Day Number (JDN, https://en.wikipedia.org/wiki/Julian_day)
##D ## is the number of days since noon UTC on the first day of 4317 BCE.
##D ## in the proleptic Julian calendar.  To more recently, in
##D ## 'Terrestrial Time' which differs from UTC by a few seconds
##D ## See https://en.wikipedia.org/wiki/Terrestrial_Time
##D julian(Sys.Date(), -2440588) # from a day
##D floor(as.numeric(julian(Sys.time())) + 2440587.5) # from a date-time
## End(No test)



