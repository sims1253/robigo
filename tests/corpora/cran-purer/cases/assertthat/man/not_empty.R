## help-page examples 'not_empty', from the pure-R CRAN package assertthat 0.2.1 (GPL-3).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(assertthat))
set.seed(1)
### Name: not_empty
### Title: Check an object doesn't have any empty dimensions
### Aliases: not_empty

### ** Examples

not_empty(numeric())
not_empty(mtcars[0, ])
not_empty(mtcars[, 0])



