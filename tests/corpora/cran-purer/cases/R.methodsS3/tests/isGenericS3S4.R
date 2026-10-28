## test script 'isGenericS3S4', from the pure-R CRAN package R.methodsS3 1.8.2 (LGPL-2.1-or-later).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(R.methodsS3))
set.seed(1)
library("R.methodsS3")

message("TESTING: isGenericS3/S4()...")

FUNs <- list(
  isGenericS3=isGenericS3,
  isGenericS4=isGenericS4
)

for (name in names(FUNs)) {
  cat(sprintf("%s():\n", name))
  FUN <- FUNs[[name]]
  print(FUN("print"))
  print(FUN("show"))
  print(FUN("unknown"))
  print(FUN(print))
  print(FUN(sum))
  print(FUN(function() NULL))
}

message("TESTING: isGenericS3/S4()...DONE")
