## help-page examples 'with_package', from the pure-R CRAN package withr 3.0.3 (MIT).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(withr))
set.seed(1)
### Name: with_package
### Title: Execute code with a modified search path
### Aliases: with_package local_package with_namespace local_namespace
###   with_environment local_environment

### ** Examples

## Not run: 
##D with_package("ggplot2", {
##D   ggplot(mtcars) + geom_point(aes(wt, hp))
##D })
## End(Not run)



