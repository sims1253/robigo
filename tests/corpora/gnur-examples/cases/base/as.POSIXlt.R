### Name: as.POSIX*
### Title: Date-time Conversion Functions
### Aliases: as.POSIXct as.POSIXct.default as.POSIXct.POSIXlt
###   as.POSIXct.date as.POSIXct.dates as.POSIXct.Date as.POSIXct.numeric
###   as.POSIXlt as.POSIXlt.Date as.POSIXlt.date as.POSIXlt.dates
###   as.POSIXlt.POSIXct as.POSIXlt.factor as.POSIXlt.character
###   as.POSIXlt.default as.POSIXlt.numeric as.double.POSIXlt
### Keywords: utilities chron

### ** Examples
## No test: 
##D (z <- Sys.time())             # the current datetime, as class "POSIXct"
##D unclass(z)                    # a large integer
##D floor(unclass(z)/86400)       # the number of days since 1970-01-01 (UTC)
##D (now <- as.POSIXlt(Sys.time())) # the current datetime, as class "POSIXlt"
##D str(unclass(now))             # the internal list ; use now$hour, etc :
##D now$year + 1900               # see ?DateTimeClasses
##D months(now); weekdays(now)    # see ?months; using LC_TIME locale
##D 
##D ## suppose we have a time in seconds since 1960-01-01 00:00:00 GMT
##D ## (the origin used by SAS)
##D z <- 1472562988
##D # ways to convert this
##D as.POSIXct(z, origin = "1960-01-01")                # local
##D as.POSIXct(z, origin = "1960-01-01", tz = "GMT")    # in UTC
##D 
##D ## SPSS dates (R-help 2006-02-16)
##D z <- c(10485849600, 10477641600, 10561104000, 10562745600)
##D as.Date(as.POSIXct(z, origin = "1582-10-14", tz = "GMT"))
##D 
##D ## Stata date-times: milliseconds since 1960-01-01 00:00:00 GMT
##D ## format %tc excludes leap-seconds, assumed here
##D ## For format %tC including leap seconds, see foreign::read.dta()
##D z <- 1579598122120
##D op <- options(digits.secs = 3)
##D # avoid rounding down: milliseconds are not exactly representable
##D as.POSIXct((z+0.1)/1000, origin = "1960-01-01")
##D options(op)
##D 
##D ## Matlab 'serial day number' (days and fractional days)
##D z <- 7.343736909722223e5 # 2010-08-23 16:35:00
##D as.POSIXct((z - 719529)*86400, origin = "1970-01-01", tz = "UTC")
##D 
##D as.POSIXlt(Sys.time(), "GMT") # the current time in UTC
## End(No test)
## No test: 
##D ## These may not be correct names on your system
##D as.POSIXlt(Sys.time(), "America/New_York")  # in New York
##D as.POSIXlt(Sys.time(), "EST5EDT")           # alternative.
##D as.POSIXlt(Sys.time(), "EST" )   # somewhere in Eastern Canada
##D as.POSIXlt(Sys.time(), "HST")    # in Hawaii
##D as.POSIXlt(Sys.time(), "Australia/Darwin")
## End(No test)

tab <- file.path(R.home("share"), "zoneinfo", "zone1970.tab")
if(file.exists(tab)) { # typically on Windows; on Linux when  `configure --with-internal-tzcode`
  cols <- c("code", "coordinates", "TZ", "comments")
  tmp <- read.delim(tab,
                    header = FALSE, comment.char = "#", col.names = cols)
  if(interactive()) View(tmp)
  head(tmp, 10)
}


