RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: weighted.residuals
### Title: Compute Weighted Residuals
### Aliases: weighted.residuals
### Keywords: regression

### ** Examples

## following on from example(lm)
## Don't show: 
utils::example("lm", echo = FALSE)
## End(Don't show)
all.equal(weighted.residuals(lm.D9),
          residuals(lm.D9))
x <- 1:10
w <- 0:9
y <- rnorm(x)
weighted.residuals(lmxy <- lm(y ~ x, weights = w))
weighted.residuals(lmxy, drop0 = FALSE)



