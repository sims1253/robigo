### Name: summary.princomp
### Title: Summary method for Principal Components Analysis
### Aliases: summary.princomp print.summary.princomp
### Keywords: multivariate

### ** Examples

summary(pc.cr <- princomp(USArrests, cor = TRUE))
## The signs of the loading columns are arbitrary
print(summary(princomp(USArrests, cor = TRUE),
              loadings = TRUE, cutoff = 0.2), digits = 2)



