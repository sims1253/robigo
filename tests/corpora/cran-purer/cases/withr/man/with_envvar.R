## help-page examples 'with_envvar', from the pure-R CRAN package withr 3.0.3 (MIT).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(withr))
set.seed(1)
### Name: with_envvar
### Title: Environment variables
### Aliases: with_envvar local_envvar

### ** Examples

with_envvar(new = c("GITHUB_PAT" = "abcdef"), Sys.getenv("GITHUB_PAT"))

# with_envvar unsets variables after usage
Sys.getenv("TEMP_SECRET")
with_envvar(new = c("TEMP_SECRET" = "secret"), Sys.getenv("TEMP_SECRET"))
Sys.getenv("TEMP_SECRET")



