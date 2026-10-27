RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: package_dependencies
### Title: Computations on the Dependency Hierarchy of Packages
### Aliases: package_dependencies
### Keywords: utilities

### ** Examples

## No test: 
##D myPkgs <- c("MASS", "Matrix", "KernSmooth", "class", "cluster", "codetools")
##D pdb <- available.packages(repos = findCRANmirror("web"))
##D system.time(
##D dep1 <- package_dependencies(myPkgs, db = pdb) # all arguments at default
##D ) # very fast
##D utils::str(dep1, vec.len=10)
##D 
##D system.time( ## reverse dependencies, recursively --- takes much longer:
##D deps <- package_dependencies(myPkgs, db = pdb, which = "most",
##D                              recursive = TRUE, reverse = TRUE)
##D ) # seen ~ 10 seconds
##D 
##D lengths(deps) # 2020-05-03: all are 16053, but codetools with 16057
##D 
##D ## install.packages(dependencies = TRUE) installs 'most' dependencies
##D ## and the strong recursive dependencies of these: these dependencies
##D ## can be obtained using 'which = "most"' and 'recursive = "strong"'.
##D ## To illustrate on the first packages with non-missing Suggests:
##D packages <- pdb[head(which(!is.na(pdb[, "Suggests"]))), "Package"]
##D package_dependencies(packages, db = pdb,
##D                      which = "most", recursive = "strong")
## End(No test)



