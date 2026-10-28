## help-page examples 'defer', from the pure-R CRAN package withr 3.0.3 (MIT).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(withr))
set.seed(1)
### Name: defer
### Title: Defer Evaluation of an Expression
### Aliases: defer defer_parent deferred_run deferred_clear

### ** Examples

# define a 'local' function that creates a file, and
# removes it when the parent function has finished executing
local_file <- function(path) {
  file.create(path)
  defer_parent(unlink(path))
}

# create tempfile path
path <- tempfile()

# use 'local_file' in a function
local({
  local_file(path)
  stopifnot(file.exists(path))
})

# file is deleted as we leave 'local' local
stopifnot(!file.exists(path))

# investigate how 'defer' modifies the
# executing function's environment
local({
  local_file(path)
  print(attributes(environment()))
})

# Note that examples lack function scoping so deferred calls are
# generally executed immediately
defer(print("one"))
defer(print("two"))



