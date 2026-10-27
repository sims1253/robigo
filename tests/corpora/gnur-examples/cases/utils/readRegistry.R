RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: readRegistry
### Title: Read a Windows Registry Hive
### Aliases: readRegistry
### Keywords: utilities

### ** Examples
## No test: 
##D if(.Platform$OS.type == "windows") withAutoprint({
##D   ## only in HLM if set in an admin-mode install.
##D   try(readRegistry("SOFTWARE\\R-core", maxdepth = 3))
##D 
##D   gmt <- file.path("SOFTWARE", "Microsoft", "Windows NT",
##D                    "CurrentVersion", "Time Zones",
##D 		   "GMT Standard Time", fsep = "\\")
##D   readRegistry(gmt, "HLM")
##D }) 
## End(No test)
## Not run: 
##D ## on a 64-bit R need this to find 32-bit JAGS
##D readRegistry("SOFTWARE\\JAGS", maxdepth = 3, view = "32")
##D 
##D ## See if there is a 64-bit user install
##D readRegistry("SOFTWARE\\R-core\\R64", "HCU", maxdepth = 2)
## End(Not run)



