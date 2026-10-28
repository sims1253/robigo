## help-page examples 'throw', from the pure-R CRAN package R.methodsS3 1.8.2 (LGPL-2.1-or-later).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(R.methodsS3))
set.seed(1)
### Name: throw
### Title: Throws an exception
### Aliases: throw.default throw
### Keywords: error internal

### ** Examples

rbern <- function(n=1, prob=1/2) {
  if (prob < 0 || prob > 1)
    throw("Argument 'prob' is out of range: ", prob)
  rbinom(n=n, size=1, prob=prob)
}

rbern(10, 0.4)
# [1] 0 1 0 0 0 1 0 0 1 0
tryCatch({
  rbern(10, 10*0.4)
}, error=function(ex) {})



