RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: system.time
### Title: CPU Time Used
### Aliases: system.time
### Keywords: utilities

### ** Examples

require(stats)
## No test: 
##D system.time(for(i in 1:100) mad(runif(1000)))
## End(No test)
## Not run: 
##D exT <- function(n = 10000) {
##D   # Purpose: Test if system.time works ok;   n: loop size
##D   system.time(for(i in 1:n) x <- mean(rt(1000, df = 4)))
##D }
##D #-- Try to interrupt one of the following (using Ctrl-C / Escape):
##D exT()                 #- about 4 secs on a 2.5GHz Xeon
##D system.time(exT())    #~ +/- same
## End(Not run)


