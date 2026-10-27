RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: princomp
### Title: Principal Components Analysis
### Aliases: princomp princomp.formula princomp.default plot.princomp
###   print.princomp predict.princomp
### Keywords: multivariate

### ** Examples

require(graphics)

## The variances of the variables in the
## USArrests data vary by orders of magnitude, so scaling is appropriate
(pc.cr <- princomp(USArrests))  # inappropriate
princomp(USArrests, cor = TRUE) # =^= prcomp(USArrests, scale=TRUE)
## Similar, but different:
## The standard deviations differ by a factor of sqrt(49/50)

summary(pc.cr <- princomp(USArrests, cor = TRUE))
loadings(pc.cr)  # note that blank entries are small but not zero
## The signs of the columns of the loadings are arbitrary
plot(pc.cr) # shows a screeplot.
biplot(pc.cr)

## Formula interface
princomp(~ ., data = USArrests, cor = TRUE)

## NA-handling
USArrests[1, 2] <- NA
pc.cr <- princomp(~ Murder + Assault + UrbanPop,
                  data = USArrests, na.action = na.exclude, cor = TRUE)
## No test: 
##D pc.cr$scores[1:5, ]
## End(No test)

## (Simple) Robust PCA:
## Classical:
(pc.cl  <- princomp(stackloss))
## No test: 
##D ## Robust:
##D (pc.rob <- princomp(stackloss, covmat = MASS::cov.rob(stackloss)))
## End(No test)


