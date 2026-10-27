### Name: gc
### Title: Garbage Collection
### Aliases: gc gcinfo
### Keywords: environment

### ** Examples
## No test: 
##D gc() #- do it now
##D gcinfo(TRUE) #-- in the future, show when R does it
##D ##            vvvvv use larger to *show* something
##D x <- integer(100000); for(i in 1:18) x <- c(x, i)
##D gcinfo(verbose = FALSE) #-- don't show it anymore
##D 
##D gc(TRUE)
##D 
##D gc(reset = TRUE)
## End(No test)


