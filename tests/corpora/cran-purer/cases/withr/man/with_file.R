## help-page examples 'with_file', from the pure-R CRAN package withr 3.0.3 (MIT).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(withr))
set.seed(1)
### Name: with_file
### Title: Files which delete themselves
### Aliases: with_file local_file

### ** Examples

with_file("file1", {
  writeLines("foo", "file1")
  readLines("file1")
})

with_file(list("file1" = writeLines("foo", "file1")), {
  readLines("file1")
})



