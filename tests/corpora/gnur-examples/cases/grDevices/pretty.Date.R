RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: pretty.Date
### Title: Pretty Breakpoints for Date-Time Classes
### Aliases: pretty.Date pretty.POSIXt
### Keywords: dplot

### ** Examples

## IGNORE_RDIFF_BEGIN
pretty(Sys.Date())
pretty(Sys.time(), n = 10)
## IGNORE_RDIFF_END
pretty(as.Date("2000-03-01")) # R 1.0.0 came in a leap year
## No test: ##D 
##D ## time ranges in diverse scales:
##D steps <- stats::setNames(,
##D     c("10 secs", "1 min", "5 mins", "30 mins", "6 hours", "12 hours",
##D       "1 DSTday", "2 weeks", "1 month", "6 months", "1 year",
##D       "10 years", "50 years", "1000 years"))
##D x <- as.POSIXct("2002-02-02 02:02")
##D lapply(steps,
##D        function(s) {
##D            at <- pretty(seq(x, by = s, length.out = 2), n = 5)
##D            attr(at, "labels")
##D        })
## End(No test)



