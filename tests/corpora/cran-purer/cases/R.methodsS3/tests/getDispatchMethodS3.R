## test script 'getDispatchMethodS3', from the pure-R CRAN package R.methodsS3 1.8.2 (LGPL-2.1-or-later).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(R.methodsS3))
set.seed(1)
library("R.methodsS3")

message("TESTING: getDispatchMethodS3()...")

fcn <- getDispatchMethodS3("print", "default")
print(fcn)

tryCatch({
  fcn <- getDispatchMethodS3("print", "unknown")
  print(fcn)
}, error = function(ex) {
  print(ex)
})

message("TESTING: getDispatchMethodS3()...DONE")
