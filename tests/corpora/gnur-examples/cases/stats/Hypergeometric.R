### Name: Hypergeometric
### Title: The Hypergeometric Distribution
### Aliases: Hypergeometric dhyper phyper qhyper rhyper
### Keywords: distribution

### ** Examples

m <- 10; n <- 7; k <- 8
x <- 0:(k+1)
rbind(phyper(x, m, n, k), dhyper(x, m, n, k))
all(phyper(x, m, n, k) == cumsum(dhyper(x, m, n, k)))  # FALSE
## No test: 
##D ## but errors are very small:
##D signif(phyper(x, m, n, k) - cumsum(dhyper(x, m, n, k)), digits = 3)
## End(No test)
stopifnot(abs(phyper(x, m, n, k) - cumsum(dhyper(x, m, n, k))) < 5e-16)



