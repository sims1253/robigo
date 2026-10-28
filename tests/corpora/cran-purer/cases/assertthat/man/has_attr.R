## help-page examples 'has_attr', from the pure-R CRAN package assertthat 0.2.1 (GPL-3).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(assertthat))
set.seed(1)
### Name: has_attr
### Title: Has attribute or name?
### Aliases: has_attr %has_attr% has_name %has_name%

### ** Examples

has_attr(has_attr, "fail")
x <- 10
x %has_attr% "a"

y <- list(a = 1, b = 2)
see_if(y %has_name% "c")



