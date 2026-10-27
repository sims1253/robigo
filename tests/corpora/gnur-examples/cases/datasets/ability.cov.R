RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: ability.cov
### Title: Ability and Intelligence Tests
### Aliases: ability.cov
### Keywords: datasets

### ** Examples
## No test: 
##D require(stats)
##D (ability.FA <- factanal(factors = 1, covmat = ability.cov))
##D update(ability.FA, factors = 2)
##D ## The signs of factors and hence the signs of correlations are
##D ## arbitrary with promax rotation.
##D update(ability.FA, factors = 2, rotation = "promax")
## End(No test)


