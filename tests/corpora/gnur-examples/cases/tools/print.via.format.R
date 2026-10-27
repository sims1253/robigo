RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: .print.via.format
### Title: Printing Utilities
### Aliases: .print.via.format
### Keywords: utilities

### ** Examples

## The function is simply defined as
 function (x, ...) {
    writeLines(format(x, ...))
    invisible(x)
 }

## is used for simple print methods in R, and as prototype for new methods.



