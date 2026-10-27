RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: lowess
### Title: Scatter Plot Smoothing
### Aliases: lowess
### Keywords: smooth

### ** Examples

require(graphics)

plot(cars, main = "lowess(cars)")
lines(lowess(cars), col = 2)
lines(lowess(cars, f = .2), col = 3)
legend(5, 120, c(paste("f = ", c("2/3", ".2"))), lty = 1, col = 2:3)



