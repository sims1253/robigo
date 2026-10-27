RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(parallel)
### Name: mcfork
### Title: Fork a Copy of the Current R Process
### Aliases: mcfork mcexit
### Keywords: interface

### ** Examples
## No test: 
##D ## This will work when run as an example, but not when pasted in.
##D p <- parallel:::mcfork()
##D if (inherits(p, "masterProcess")) {
##D     cat("I'm a child! ", Sys.getpid(), "\n")
##D     parallel:::mcexit(,"I was a child")
##D }
##D cat("I'm the master\n")
##D unserialize(parallel:::readChildren(1.5))
## End(No test)


