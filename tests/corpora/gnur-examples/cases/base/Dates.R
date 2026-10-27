RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Dates
### Title: Date Class
### Aliases: Date Dates print.Date summary.Date Math.Date Summary.Date
###   [.Date [<-.Date length<-.Date [[.Date as.list.Date c.Date mean.Date
###   split.Date
### Keywords: utilities chron

### ** Examples
## No test: 
##D (today <- Sys.Date())
##D format(today, "%d %b %Y")  # with month as a word
##D (tenweeks <- seq(today, length.out=10, by="1 week")) # next ten weeks
##D weekdays(today)
##D months(tenweeks)
## End(No test)
(Dls <- as.Date(.leap.seconds))

## Show use of year zero:
(z <- as.Date("01-01-01")) # how it is printed depends on the OS
z - 365 # so year zero was a leap year.
as.Date("00-02-29")
# if you want a different format, consider something like (if supported)
## Not run: 
##D format(z, "%04Y-%m-%d") # "0001-01-01"
##D format(z, "%_4Y-%m-%d") # "   1-01-01"
##D format(z, "%_Y-%m-%d")  # "1-01-01"
## End(Not run)
 

##  length(<Date>) <- n   now works
ls <- Dls; length(ls) <- 12
l2 <- Dls; length(l2) <- 5 + length(Dls)
stopifnot(exprs = {
  ## length(.) <- * is compatible to subsetting/indexing:
  identical(ls, Dls[seq_along(ls)])
  identical(l2, Dls[seq_along(l2)])
  ## has filled with NA's
  is.na(l2[(length(Dls)+1):length(l2)])
})



