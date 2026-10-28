## help-page examples 'setGenericS3', from the pure-R CRAN package R.methodsS3 1.8.2 (LGPL-2.1-or-later).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(R.methodsS3))
set.seed(1)
### Name: setGenericS3
### Title: Creates an S3 generic function
### Aliases: setGenericS3.default setGenericS3
### Keywords: programming methods

### ** Examples

myCat.matrix <- function(..., sep=", ") {
  cat("A matrix:\n")
  cat(..., sep=sep)
  cat("\n")
}

myCat.default <- function(..., sep=", ") {
  cat(..., sep=sep)
  cat("\n")
}

setGenericS3("myCat")

myCat(1:10)
mat <- matrix(1:10, ncol=5)
myCat(mat)



