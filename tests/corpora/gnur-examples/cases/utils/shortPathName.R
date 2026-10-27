RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: shortPathName
### Title: Express File Paths in Short Form on Windows
### Aliases: shortPathName
### Keywords: utilities

### ** Examples

if(.Platform$OS.type == "windows") withAutoprint({

## No test: 
##D   cat(shortPathName(c(R.home(), tempdir())), sep = "\n")
## End(No test)
## Don't show: 
  cat(shortPathName(R.home()), sep = "\n")
## End(Don't show)
})



