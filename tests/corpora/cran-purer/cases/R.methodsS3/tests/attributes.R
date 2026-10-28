## test script 'attributes', from the pure-R CRAN package R.methodsS3 1.8.2 (LGPL-2.1-or-later).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(R.methodsS3))
set.seed(1)
library("R.methodsS3")

message("TESTING: attributes()...")

export <- R.methodsS3:::export
`export<-` <- R.methodsS3:::`export<-`
noexport <- R.methodsS3:::noexport
`S3class<-` <- R.methodsS3:::`S3class<-`


foo <- function() NULL
str(foo)

foo <- export(foo)
str(foo)

export(foo) <- TRUE
str(foo)

foo <- noexport(foo)
str(foo)

foo.Bar <- function(...) NULL
S3class(foo.Bar) <- "Bar"
str(foo)

message("TESTING: attributes()...DONE")
