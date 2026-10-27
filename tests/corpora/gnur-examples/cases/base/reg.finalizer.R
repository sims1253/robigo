RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: reg.finalizer
### Title: Finalization of Objects
### Aliases: reg.finalizer finalizer
### Keywords: programming environment

### ** Examples

f <- function(e) print("cleaning....")
g <- function(x){ e <- environment(); reg.finalizer(e, f) }
g()
invisible(gc()) # trigger cleanup



