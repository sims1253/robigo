### Name: as.hclust
### Title: Convert Objects to Class '"hclust"'
### Aliases: as.hclust as.hclust.default as.hclust.twins
### Keywords: multivariate cluster

### ** Examples

x <- matrix(rnorm(30), ncol = 3)
hc <- hclust(dist(x), method = "complete")

## No test: 
##D if(require("cluster", quietly = TRUE)) {# is a recommended package
##D   ag <- agnes(x, method = "complete")
##D   hcag <- as.hclust(ag)
##D   ## The dendrograms order slightly differently:
##D   op <- par(mfrow = c(1,2))
##D   plot(hc) ;  mtext("hclust", side = 1)
##D   plot(hcag); mtext("agnes",  side = 1)
##D   detach("package:cluster")
##D }
## End(No test)



