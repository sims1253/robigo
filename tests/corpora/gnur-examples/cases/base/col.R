RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: col
### Title: Column Indexes
### Aliases: col .col
### Keywords: array

### ** Examples

# extract an off-diagonal of a matrix
ma <- matrix(1:12, 3, 4)
ma[row(ma) == col(ma) + 1]

# create an identity 5-by-5 matrix more slowly than diag(n = 5):
x <- matrix(0, nrow = 5, ncol = 5)
x[row(x) == col(x)] <- 1

(i34 <- .col(3:4))
stopifnot(identical(i34, .col(c(3,4)))) # 'dim' maybe "double"



