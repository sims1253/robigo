RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: check_packages_in_dir
### Title: Check Source Packages and Their Reverse Dependencies
### Aliases: check_packages_in_dir summarize_check_packages_in_dir_depends
###   summarize_check_packages_in_dir_results
###   summarize_check_packages_in_dir_timings check_packages_in_dir_changes
###   check_packages_in_dir_details
### Keywords: utilities

### ** Examples

## Not run: 
##D ## Check packages in dir without reverse dependencies:
##D check_packages_in_dir(dir)
##D ## Check packages in dir and their reverse dependencies using the
##D ## defaults (all repositories in getOption("repos"), all "strong"
##D ## reverse dependencies, no recursive reverse dependencies):
##D check_packages_in_dir(dir, reverse = list())
##D ## Check packages in dir with their reverse dependencies from CRAN,
##D ## using all strong reverse dependencies and reverse suggests:
##D check_packages_in_dir(dir,
##D                       reverse = list(repos = getOption("repos")["CRAN"],
##D                                      which = "most"))                   
##D ## Check packages in dir with their reverse dependencies from CRAN,
##D ## using '--as-cran' for the former but not the latter:
##D check_packages_in_dir(dir,
##D                       check_args = c("--as-cran", ""),
##D                       reverse = list(repos = getOption("repos")["CRAN"]))
## End(Not run)



