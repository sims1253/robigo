RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: svd
### Title: Singular Value Decomposition of a Matrix
### Aliases: svd La.svd
### Keywords: algebra array

### ** Examples

hilbert <- function(n) { i <- 1:n; 1 / outer(i - 1, i, `+`) }
X <- hilbert(9)[, 1:6]
(s <- svd(X))
D <- diag(s$d)
s$u %*% D %*% t(s$v) #  X = U D V'
t(s$u) %*% X %*% s$v #  D = U' X V



