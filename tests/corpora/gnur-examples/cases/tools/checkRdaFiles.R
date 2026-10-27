RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: checkRdaFiles
### Title: Report on Details of Saved Images or Re-saves them
### Aliases: checkRdaFiles resaveRdaFiles
### Keywords: utilities

### ** Examples
## Not run: 
##D ## from a package top-level source directory
##D paths <- sort(Sys.glob(c("data/*.rda", "data/*.RData")))
##D (res <- checkRdaFiles(paths))
##D ## pick out some that may need attention
##D bad <- is.na(res$ASCII) | res$ASCII | (res$size > 1e4 & res$compress == "none")
##D res[bad, ]
## End(Not run)


