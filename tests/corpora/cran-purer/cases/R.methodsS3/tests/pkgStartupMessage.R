## test script 'pkgStartupMessage', from the pure-R CRAN package R.methodsS3 1.8.2 (LGPL-2.1-or-later).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(R.methodsS3))
set.seed(1)
library("R.methodsS3")

message("TESTING: pkgStartupMessage()...")

msg <- "Hello world!"
pkgStartupMessage(msg)

for (quietly in c(NA, FALSE, TRUE)) {
  msg <- sprintf("Hello world! (quietly=%s)", quietly)
  pkgStartupMessage(msg, quietly=quietly)
}

message("TESTING: pkgStartupMessage()...DONE")
