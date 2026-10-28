## help-page examples 'with_language', from the pure-R CRAN package withr 3.0.3 (MIT).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(withr))
set.seed(1)
### Name: with_language
### Title: Language
### Aliases: with_language local_language

### ** Examples

with_language("en", try(mean[[1]]))
with_language("fr", try(mean[[1]]))
with_language("es", try(mean[[1]]))



