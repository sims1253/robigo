RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: file.info
### Title: Extract File Information
### Aliases: file.info file.mode file.mtime file.size
### Keywords: file

### ** Examples

ncol(finf <- file.info(dir()))  # at least six
## No test: 
##D finf # the whole list
## End(No test)
## Those that are more than 100 days old :
finf <- file.info(dir(), extra_cols = FALSE)
finf[difftime(Sys.time(), finf[,"mtime"], units = "days") > 100 , 1:4]

file.info("no-such-file-exists")

## No test: 
##D ## E.g., for R-core, in a R-devel version:
##D if(Sys.info()[["sysname"]] == "Linux") 
##D     sort(file.mtime(file.path(R.home("bin"),
##D                              c("",
##D                                file.path(c("", "exec"), "R")))
##D          ))
## End(No test)


