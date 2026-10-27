RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Sys.time
### Title: Get Current Date and Time
### Aliases: Sys.time Sys.Date
### Keywords: utilities chron

### ** Examples
## No test: 
##D Sys.time()
##D ## print with possibly greater accuracy:
##D op <- options(digits.secs = 6)
##D Sys.time()
##D options(op)
##D 
##D ## locale-specific version of date()
##D format(Sys.time(), "%a %b %d %X %Y")
##D 
##D Sys.Date()
## End(No test)


