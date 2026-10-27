RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: strptime
### Title: Date-time Conversion Functions to and from Character
### Aliases: format.POSIXct format.POSIXlt strftime strptime
###   as.character.POSIXt
### Keywords: utilities chron

### ** Examples
## No test: 
##D ## locale-specific version of date()
##D format(Sys.time(), "%a %b %d %X %Y %Z")
##D 
##D ## time to sub-second accuracy (if supported by the OS)
##D format(Sys.time(), "%H:%M:%OS3")
## End(No test)
## read in date info in format 'ddmmmyyyy'
## This will give NA(s) in some non-English locales; setting the C locale
## as in the commented lines will overcome this on most systems.
## lct <- Sys.getlocale("LC_TIME"); Sys.setlocale("LC_TIME", "C")
x <- c("1jan1960", "2jan1960", "31mar1960", "30jul1960")
z <- strptime(x, "%d%b%Y")
## Sys.setlocale("LC_TIME", lct)
## No test: 
##D z
## End(No test)
(chz <- as.character(z)) # same w/o TZ
## *here* (but not in general), the same as format():
stopifnot(exprs = {
     identical(chz, format(z))
     grepl("^1960-0[137]-[03][012]$", chz[!is.na(z)])
})

## read in date/time info in format 'm/d/y h:m:s'
dates <- c("02/27/92", "02/27/92", "01/14/92", "02/28/92", "02/01/92")
times <- c("23:03:20", "22:29:56", "01:03:30", "18:21:03", "16:56:26")
x <- paste(dates, times)
z2 <- strptime(x, "%m/%d/%y %H:%M:%S")
## No test: 
##D z2 
## End(No test)
## *here* (but not in general), the same as format():
stopifnot(identical(format(z2), as.character(z2)))

## time with fractional seconds (setting `tz = ..` for reproducible output)
z3 <- strptime("20/2/06 11:16:16.683", "%d/%m/%y %H:%M:%OS", tz = "UTC")
z3 # prints without fractional seconds by default, digits.sec = NULL ("= 0")
format(z3, digits = 3) # shows extra digits
format(z3, digits = 6) # still 3 digits: *not* showing trailing zeros
format(z3, format = "%Y-%m-%d %H:%M:%OS6") #  *does* keep trailing zeros
op <- options(digits.secs = 3) # global option, the default for `digits`
z3 # shows the 3 extra digits
options(op)
as.character(z3) # ditto


## time zone names are not portable, but 'EST5EDT' comes pretty close.
## (but its interpretation may not be universal: see ?timezones)
z4 <- strptime(c("2006-01-08 10:07:52", "2006-08-07 19:33:02"),
               "%Y-%m-%d %H:%M:%S", tz = "EST5EDT")
## No test: 
##D z4 
## End(No test)
attr(z4, "tzone")
as.character(z4)
z4$sec[2] <- pi # "very" fractional seconds
as.character(z4) # shows full precision
format(z4) # no fractional sec
format(z4, digits=8) # shows only 6  (hard-wired maximum)
format(z4, digits=4)


## An RFC 5322 header (Eastern Canada, during DST)
## In a non-English locale the commented lines may be needed.
## No test: 
##D ## prev <- Sys.getlocale("LC_TIME"); Sys.setlocale("LC_TIME", "C")
##D strptime("Tue, 23 Mar 2010 14:36:38 -0400", "%a, %d %b %Y %H:%M:%S %z")
##D ## Sys.setlocale("LC_TIME", prev)
##D 
##D ## Make sure you know what the abbreviated names are for you if you wish
##D ## to use them for input (they are matched case-insensitively):
##D format(s1 <- seq.Date(as.Date('1978-01-01'), by = 'day',   len =  7), "%a")
##D format(s2 <- seq.Date(as.Date('2000-01-01'), by = 'month', len = 12), "%b")
## End(No test)
## Non-finite date-times :
format(as.POSIXct(Inf)) # "Inf"  (was  NA  in R <= 4.1.x)
format(as.POSIXlt(c(-Inf,Inf,NaN,NA))) # were all NA



