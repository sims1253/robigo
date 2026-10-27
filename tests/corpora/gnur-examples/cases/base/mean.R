RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: mean
### Title: Arithmetic Mean
### Aliases: mean mean.default
### Keywords: univar

### ** Examples

x <- c(0:10, 50)
xm <- mean(x)
c(xm, mean(x, trim = 0.10))



