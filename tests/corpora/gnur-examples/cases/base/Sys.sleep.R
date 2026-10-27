RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Sys.sleep
### Title: Suspend Execution for a Time Interval
### Aliases: Sys.sleep
### Keywords: utilities

### ** Examples
## No test: 
##D testit <- function(x)
##D {
##D     p1 <- proc.time()
##D     Sys.sleep(x)
##D     proc.time() - p1 # The cpu usage should be negligible
##D }
##D testit(3.7)
## End(No test)


