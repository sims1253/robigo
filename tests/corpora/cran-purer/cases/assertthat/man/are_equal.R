## help-page examples 'are_equal', from the pure-R CRAN package assertthat 0.2.1 (GPL-3).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(assertthat))
set.seed(1)
### Name: are_equal
### Title: Are two objects equal?
### Aliases: are_equal

### ** Examples

x <- 2
see_if(are_equal(x, 1.9))
see_if(are_equal(x, 1.999, tol = 0.01))
see_if(are_equal(x, 2))



