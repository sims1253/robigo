## help-page examples 'with_path', from the pure-R CRAN package withr 3.0.3 (MIT).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(withr))
set.seed(1)
### Name: with_path
### Title: PATH environment variable
### Aliases: with_path local_path

### ** Examples

# temporarily modify the system PATH, *prefixing* the current path
with_path(getwd(), Sys.getenv("PATH"))
# temporarily modify the system PATH, *appending* to the current path
with_path(getwd(), Sys.getenv("PATH"), "suffix")



