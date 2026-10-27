### Name: R.Version
### Title: Version Information
### Aliases: R.Version R.version version R.version.string R_compiled_by
### Keywords: environment sysdata programming

### ** Examples

require(graphics)

R.version$os # to check how lucky you are ...
plot(0) # any plot
mtext(R.version.string, side = 1, line = 4, adj = 1) # a useful bottom-right note

## a good way to detect macOS:
if(grepl("^darwin", R.version$os)) message("running on macOS")

## Short R version string, ("space free", useful in file/directory names;
##                          also fine for unreleased versions of R):
shortRversion <- function() {
   rvs <- R.version.string
   if(grepl("devel", (st <- R.version$status)))
       rvs <- sub(paste0(" ",st," "), "-devel_", rvs, fixed=TRUE)
   gsub("[()]", "", gsub(" ", "_", sub(" version ", "-", rvs)))
}
## No test: 
##D shortRversion()
## End(No test)
## Don't show: 
stopifnot(identical(grep(" ", shortRversion()), integer(0)))
## End(Don't show)



