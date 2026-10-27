library(tools)
### Name: makevars
### Title: User and Site Compilation Variables
### Aliases: makevars_user makevars_site
### Keywords: utilities

### ** Examples
## No test: 
##D makevars_user()
##D makevars_site()
## End(No test)
## Don't show: 
checkMV <- function(r)
  stopifnot(is.character(r),
            length(r) == 0 || (length(r) == 1 && file.exists(r)))
checkMV(makevars_user())
checkMV(makevars_site())
## End(Don't show)



