RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Sys.getpid
### Title: Get the Process ID of the R Session
### Aliases: Sys.getpid
### Keywords: utilities

### ** Examples
## No test: 
##D Sys.getpid()
##D 
##D ## Show files opened from this R process
##D if(.Platform$OS.type == "unix") ## on Unix-alikes such Linux, macOS, FreeBSD:
##D    system(paste("lsof -p", Sys.getpid()))
## End(No test)


