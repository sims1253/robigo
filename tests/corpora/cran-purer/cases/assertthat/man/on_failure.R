## help-page examples 'on_failure', from the pure-R CRAN package assertthat 0.2.1 (GPL-3).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(assertthat))
set.seed(1)
### Name: on_failure
### Title: Custom failure messages for assertions.
### Aliases: on_failure on_failure<-

### ** Examples

is_odd <- function(x) {
  assert_that(is.numeric(x), length(x) == 1)
  x %% 2 == 1
}
see_if(is_odd(2))

on_failure(is_odd) <- function(call, env) {
  paste0(deparse(call$x), " is even")
}
see_if(is_odd(2))



