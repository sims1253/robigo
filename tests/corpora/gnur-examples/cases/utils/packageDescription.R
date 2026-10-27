### Name: packageDescription
### Title: Package Description
### Aliases: packageDescription packageVersion packageDate asDateBuilt
###   print.packageDescription
### Keywords: utilities

### ** Examples
## No test: 
##D packageDescription("stats")
##D packageDescription("stats", fields = c("Package", "Version"))
##D 
##D packageDescription("stats", fields = "Version")
##D packageDescription("stats", fields = "Version", drop = FALSE)
## End(No test)
if(requireNamespace("MASS") && packageVersion("MASS") < "7.3.29")
  message("you need to update 'MASS'")

pu <- packageDate("utils")
## No test: 
##D str(pu)
## End(No test)
stopifnot(identical(pu, packageDate(desc = packageDescription("utils"))),
          identical(pu, packageDate("stats"))) # as "utils" and "stats" are
                                   # both 'base R' and "Built" at same time



