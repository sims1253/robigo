RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: testInstalledPackage
### Title: Test Installed Packages
### Aliases: testInstalledPackage testInstalledPackages testInstalledBasic
###   TEST_MC_CORES standard_package_names
### Keywords: utilities

### ** Examples

str(stPkgs <- standard_package_names())
## No test: ##D 
##D ## consistency of packageDescription and standard_package_names :
##D (pNms <- unlist(stPkgs, FALSE))
##D (prio <- sapply(as.vector(pNms), packageDescription, fields = "Priority"))
##D stopifnot(identical(unname(prio),
##D                     sub("[0-9]+$", '', names(pNms))))
## End(No test)



