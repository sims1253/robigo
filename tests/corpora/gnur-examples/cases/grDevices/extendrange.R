RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: extendrange
### Title: Extend a Numerical Range by a Small Percentage
### Aliases: extendrange
### Keywords: dplot

### ** Examples

x <- 1:5
(r <- range(x))         # 1    5
extendrange(x)          # 0.8  5.2
extendrange(x, f= 0.01) # 0.96 5.04

## extend more to the right:
extendrange(x, f=c(.01,.03)) # 0.96 5.12

## Use 'r' if you have it already:
stopifnot(identical(extendrange(r = r),
                    extendrange(x)))



