RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: is.empty.model
### Title: Test if a Model's Formula is Empty
### Aliases: is.empty.model
### Keywords: models

### ** Examples

y <- rnorm(20)
is.empty.model(y ~ 0)
is.empty.model(y ~ -1)
is.empty.model(lm(y ~ 0))



