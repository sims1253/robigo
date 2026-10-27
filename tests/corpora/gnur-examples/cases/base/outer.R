RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: outer
### Title: Outer Product of Arrays
### Aliases: outer %o%
### Keywords: array

### ** Examples

x <- 1:9; names(x) <- x
# Multiplication & Power Tables
x %o% x
y <- 2:8; names(y) <- paste(y,":", sep = "")
outer(y, x, `^`)

outer(month.abb, 1999:2003, FUN = paste)

## three way multiplication table:
x %o% x %o% y[1:3]



