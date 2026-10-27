RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: find.package
### Title: Find Packages
### Aliases: find.package path.package packageNotFoundError
### Keywords: file

### ** Examples

try(find.package("knitr"))
## will not give an error, maybe a warning about *all* locations it is found:
find.package("kitty", quiet=TRUE, verbose=TRUE)

## Find all .libPaths() entries a package is found:
findPkgAll <- function(pkg)
  unlist(lapply(.libPaths(), function(lib)
           find.package(pkg, lib, quiet=TRUE, verbose=FALSE)))

findPkgAll("MASS")
findPkgAll("knitr")



