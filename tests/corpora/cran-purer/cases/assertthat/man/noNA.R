## help-page examples 'noNA', from the pure-R CRAN package assertthat 0.2.1 (GPL-3).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(assertthat))
set.seed(1)
### Name: noNA
### Title: Does object contain any missing values?
### Aliases: noNA

### ** Examples

see_if(noNA("a"))
see_if(noNA(c(TRUE, NA)))
x <- sample(c(1:10, NA), 100, rep = TRUE)
see_if(noNA(x))



