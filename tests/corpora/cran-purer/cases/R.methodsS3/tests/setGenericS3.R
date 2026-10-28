## test script 'setGenericS3', from the pure-R CRAN package R.methodsS3 1.8.2 (LGPL-2.1-or-later).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(R.methodsS3))
set.seed(1)
library("R.methodsS3")

message("TESTING: setGenericS3()...")

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

setGenericS3("foo", validators=list(R.methodsS3:::rccValidateSetGenericS3))
setGenericS3("foo<-")

bar.default <- function(...) cat("bar.default\n")
bar <- function(...) cat("bar\n")
res <- tryCatch(setGenericS3("bar"), error = identity)
stopifnot(inherits(res, "error"))

print(getGenericS3("print"))


# Your definition will redefine bar() above to bar.default().
foobar <- function() print("foobar()")
setGenericS3("foobar")


# Cleanup
rm(list=ls())

message("TESTING: setGenericS3()...DONE")
