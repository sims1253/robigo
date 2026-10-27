### Name: date
### Title: System Date and Time
### Aliases: date
### Keywords: utilities

### ** Examples
## IGNORE_RDIFF_BEGIN
(d <- date())
nchar(d) == 24

## something similar in the current locale
##   depending on ctime; e.g. %e could be %d:
format(Sys.time(), "%a %b %e %H:%M:%S %Y")
## IGNORE_RDIFF_END


