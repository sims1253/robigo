RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: plot.acf
### Title: Plot Autocovariance and Autocorrelation Functions
### Aliases: plot.acf
### Keywords: hplot ts

### ** Examples

require(graphics)

z4  <- ts(matrix(rnorm(400), 100, 4), start = c(1961, 1), frequency = 12)
z7  <- ts(matrix(rnorm(700), 100, 7), start = c(1961, 1), frequency = 12)
acf(z4)
acf(z7, max.mfrow = 7)   # squeeze onto 1 page
acf(z7) # multi-page



