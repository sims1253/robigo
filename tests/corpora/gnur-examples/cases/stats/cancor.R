### Name: cancor
### Title: Canonical Correlations
### Aliases: cancor
### Keywords: multivariate

### ** Examples
## No test: 
##D ## signs of results are random
##D pop <- LifeCycleSavings[, 2:3]
##D oec <- LifeCycleSavings[, -(2:3)]
##D cancor(pop, oec)
##D 
##D x <- matrix(rnorm(150), 50, 3)
##D y <- matrix(rnorm(250), 50, 5)
##D (cxy <- cancor(x, y))
##D all(abs(cor(x %*% cxy$xcoef,
##D             y %*% cxy$ycoef)[,1:3] - diag(cxy $ cor)) < 1e-15)
##D all(abs(cor(x %*% cxy$xcoef) - diag(3)) < 1e-15)
##D all(abs(cor(y %*% cxy$ycoef) - diag(5)) < 1e-15)
## End(No test)


