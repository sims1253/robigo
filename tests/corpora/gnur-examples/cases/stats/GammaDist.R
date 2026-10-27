### Name: GammaDist
### Title: The Gamma Distribution
### Aliases: GammaDist dgamma pgamma qgamma rgamma
### Keywords: distribution

### ** Examples

-log(dgamma(1:4, shape = 1))
p <- (1:9)/10
pgamma(qgamma(p, shape = 2), shape = 2)
1 - 1/exp(qgamma(p, shape = 1))

## No test: 
##D # even for shape = 0.001 about half the mass is on numbers
##D # that cannot be represented accurately (and most of those as zero)
##D pgamma(.Machine$double.xmin, 0.001)
##D pgamma(5e-324, 0.001)  # on most machines 5e-324 is the smallest
##D                        # representable non-zero number
##D table(rgamma(1e4, 0.001) == 0)/1e4
## End(No test)


