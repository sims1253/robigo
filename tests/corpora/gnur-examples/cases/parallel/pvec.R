RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(parallel)
### Name: pvec
### Title: Parallelize a Vector Map Function using Forking
### Aliases: pvec
### Keywords: interface

### ** Examples

x <- pvec(1:1000, sqrt)
stopifnot(all(x == sqrt(1:1000)))

## No test: 
##D # One use is to convert date strings to unix time in large datasets
##D # as that is a relatively slow operation.
##D # So let's get some random dates first
##D # (A small test only with 2 cores: set options("mc.cores")
##D # and increase N for a larger-scale test.)
##D N <- 1e5
##D dates <- sprintf('%04d-%02d-%02d', as.integer(2000+rnorm(N)),
##D                  as.integer(runif(N, 1, 12)), as.integer(runif(N, 1, 28)))
##D 
##D system.time(a <- as.POSIXct(dates))
##D 
##D # But specifying the format is faster
##D system.time(a <- as.POSIXct(dates, format = "%Y-%m-%d"))
##D 
##D # pvec ought to be faster, but system overhead can be high
##D system.time(b <- pvec(dates, as.POSIXct, format = "%Y-%m-%d"))
##D stopifnot(all(a == b))
##D 
##D # using mclapply for this would much slower because each value
##D # will require a separate call to as.POSIXct()
##D # as lapply(dates, as.POSIXct) does
##D system.time(c <- unlist(mclapply(dates, as.POSIXct,  format = "%Y-%m-%d")))
##D stopifnot(all(a == c))
## End(No test)


