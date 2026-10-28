## help-page examples 'assert-is', from the pure-R CRAN package assertthat 0.2.1 (GPL-3).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(assertthat))
set.seed(1)
### Name: assert-is
### Title: Missing is functions.
### Aliases: is.error is.time is.date

### ** Examples

a <- Sys.time()
is.time(a)
b <- Sys.Date()
is.date(b)
c <- try(stop("!!"))
is.error(c)



