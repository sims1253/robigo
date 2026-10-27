RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: help
### Title: Documentation
### Aliases: help
### Keywords: documentation

### ** Examples

help()
help(help)              # the same

help(lapply)

help("for")             # or ?"for", but quotes/backticks are needed

## No test: 
##D try({# requires working TeX installation:
##D  help(dgamma, help_type = "pdf")
##D  ## -> nicely formatted pdf -- including math formula -- for help(dgamma):
##D  system2(getOption("pdfviewer"), "dgamma.pdf", wait = FALSE)
##D })
## End(No test)
## No test: 
##D help(package = "splines") # get help even when package is not loaded
## End(No test)

topi <- "women"
help(topi)

try(help("bs", try.all.packages = FALSE)) # reports not found (an error)
help("bs", try.all.packages = TRUE)       # reports can be found
                                          # in package 'splines'

## No test: 
##D ## For programmatic use:
##D topic <- "family"; pkg_ref <- "stats"
##D help((topic), (pkg_ref))
## End(No test)


