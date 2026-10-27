RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Rscript
### Title: Scripting Front-End for R
### Aliases: Rscript
### Keywords: utilities

### ** Examples
## Not run: 
##D Rscript -e 'date()' -e 'format(Sys.time(), "%a %b %d %X %Y")'
##D 
##D 
##D # Get the same initial packages in the same order as default R:
##D Rscript --default-packages=methods,datasets,utils,grDevices,graphics,stats -e 'sessionInfo()'
##D 
##D ## example #! script for a Unix-alike
##D ## (arguments given on the #! line end up as [options] to Rscript, while
##D ## arguments passed to the #! script end up as [args], so available to
##D ## commandArgs())
##D #! /path/to/Rscript --vanilla --default-packages=utils
##D args <- commandArgs(TRUE)
##D res <- try(install.packages(args))
##D if(inherits(res, "try-error")) q(status=1) else q()
##D 
## End(Not run)


