## help-page examples 'has_args', from the pure-R CRAN package assertthat 0.2.1 (GPL-3).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(assertthat))
set.seed(1)
### Name: has_args
### Title: Check a function has specified arguments
### Aliases: has_args %has_args%

### ** Examples

has_args(mean, "x")
has_args(mean, "x", exact = TRUE)

see_if(mean %has_args% "x")
see_if(mean %has_args% "y")



