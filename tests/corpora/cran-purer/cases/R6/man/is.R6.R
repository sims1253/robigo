## help-page examples 'is.R6', from the pure-R CRAN package R6 2.6.1 (MIT).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(R6))
set.seed(1)
### Name: is.R6
### Title: Is an object an R6 Class Generator or Object?
### Aliases: is.R6 is.R6Class

### ** Examples

class_generator <- R6Class()
object <- class_generator$new()

is.R6Class(class_generator)
is.R6(class_generator)

is.R6Class(object)
is.R6(object)



