## test script 'findDispatchMethodsS3', from the pure-R CRAN package R.methodsS3 1.8.2 (LGPL-2.1-or-later).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(R.methodsS3))
set.seed(1)
library("R.methodsS3")

message("TESTING: findDispatchMethodS3()...")

## Odds and ends
# Trying to retrieve base::.Options, but should be
# detected as a non-function and return an empty result
fcn <- findDispatchMethodsS3("", "Options")
stopifnot(length(fcn) == 0L)

message("TESTING: findDispatchMethodS3()...DONE")
