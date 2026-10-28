## help-page examples 'validate_that', from the pure-R CRAN package assertthat 0.2.1 (GPL-3).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(assertthat))
set.seed(1)
### Name: validate_that
### Title: Validate that certain conditions are true.
### Aliases: validate_that

### ** Examples

x <- 1
# assert_that() generates errors, so can't be usefully run in
# examples
validate_that(is.numeric(x))
validate_that(is.character(x))
validate_that(length(x) == 3)
validate_that(is.dir("asdf"))



