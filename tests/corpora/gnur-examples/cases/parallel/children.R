RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(parallel)
### Name: mcchildren
### Title: Low-level Functions for Management of Forked Processes
### Aliases: children readChild readChildren selectChildren sendChildStdin
###   sendMaster mckill
### Keywords: interface

### ** Examples
## Not run: 
##D p  <- mcparallel(scan(n = 1, quiet = TRUE))
##D sendChildStdin(p, "17.4\n")
##D mccollect(p)[[1]]
## End(Not run)


