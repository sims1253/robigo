### Name: available.packages
### Title: List Available Packages at CRAN-like Repositories
### Aliases: available.packages R_AVAILABLE_PACKAGES_CACHE_CONTROL_MAX_AGE
### Keywords: utilities

### ** Examples
## No test: 
##D ## Count package licenses
##D db <- available.packages(repos = findCRANmirror("web"), filters = "duplicates")
##D table(db[,"License"])
##D 
##D ## Use custom filter function to only keep recommended packages
##D ## which do not require compilation
##D available.packages(repos = findCRANmirror("web"),
##D   filters = list(
##D     add = TRUE,
##D     function (db) db[db[,"Priority"] %in% "recommended" &
##D                      db[,"NeedsCompilation"] == "no", ]
##D   ))
## End(No test)
## Not run: 
##D ## Restrict install.packages() (etc) to known-to-be-FOSS packages
##D options(available_packages_filters =
##D   c("R_version", "OS_type", "subarch", "duplicates", "license/FOSS"))
##D ## or
##D options(available_packages_filters = list(add = TRUE, "license/FOSS"))
##D 
##D ## Give priority to released versions on CRAN, rather than development
##D ## versions on R-Forge etc.
##D options(available_packages_filters =
##D      c("R_version", "OS_type", "subarch", "CRAN", "duplicates"))
## End(Not run)


