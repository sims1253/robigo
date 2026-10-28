## test script 'appendVarArgs', from the pure-R CRAN package R.methodsS3 1.8.2 (LGPL-2.1-or-later).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(R.methodsS3))
set.seed(1)
library("R.methodsS3")

message("TESTING: appendVarArgs()...")

foobar <- function(a=1) print(a)
print(foobar)

foobar <- appendVarArgs(foobar)
print(foobar)

foobar <- appendVarArgs(foobar)
print(foobar)

# Cleanup
rm(list=ls())

message("TESTING: appendVarArgs()...done")
