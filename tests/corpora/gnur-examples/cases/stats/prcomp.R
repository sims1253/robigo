RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: prcomp
### Title: Principal Components Analysis
### Aliases: prcomp prcomp.formula prcomp.default plot.prcomp
###   predict.prcomp print.prcomp summary.prcomp print.summary.prcomp
### Keywords: multivariate

### ** Examples

C <- chol(S <- toeplitz(.9 ^ (0:31))) # Cov.matrix and its root
all.equal(S, crossprod(C))
set.seed(17)
X <- matrix(rnorm(32000), 1000, 32)
Z <- X %*% C  ## ==>  cov(Z) ~=  C'C = S
all.equal(cov(Z), S, tolerance = 0.08)
pZ <- prcomp(Z, tol = 0.1)
summary(pZ) # only ~14 PCs (out of 32)
## or choose only 3 PCs more directly:
pz3 <- prcomp(Z, rank. = 3)
summary(pz3) # same numbers as the first 3 above
stopifnot(ncol(pZ$rotation) == 14, ncol(pz3$rotation) == 3,
          all.equal(pz3$sdev, pZ$sdev, tolerance = 1e-15)) # exactly equal typically

## No test: 
##D ## signs are random
##D require(graphics)
##D ## the variances of the variables in the
##D ## USArrests data vary by orders of magnitude, so scaling is appropriate
##D prcomp(USArrests)  # inappropriate
##D prcomp(USArrests, scale. = TRUE)
##D prcomp(~ Murder + Assault + Rape, data = USArrests, scale. = TRUE)
##D plot(prcomp(USArrests))
##D summary(prcomp(USArrests, scale. = TRUE))
##D biplot(prcomp(USArrests, scale. = TRUE))
## End(No test)



