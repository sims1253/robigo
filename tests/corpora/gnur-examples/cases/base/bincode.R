RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: .bincode
### Title: Bin a Numeric Vector
### Aliases: .bincode
### Keywords: category

### ** Examples

## An example with non-unique breaks:
x <- c(0, 0.01, 0.5, 0.99, 1)
b <- c(0, 0, 1, 1)
.bincode(x, b, TRUE)
.bincode(x, b, FALSE)
.bincode(x, b, TRUE, TRUE)
.bincode(x, b, FALSE, TRUE)



