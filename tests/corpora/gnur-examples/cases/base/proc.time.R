RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: proc.time
### Title: Running Time of R
### Aliases: proc.time print.proc_time summary.proc_time
### Keywords: utilities

### ** Examples
## No test: 
##D ## a way to time an R expression: system.time is preferred
##D ptm <- proc.time()
##D for (i in 1:50) mad(stats::runif(500))
##D proc.time() - ptm
## End(No test)


