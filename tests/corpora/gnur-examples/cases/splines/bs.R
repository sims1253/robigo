library(splines)
### Name: bs
### Title: B-Spline Basis for Polynomial Splines
### Aliases: bs
### Keywords: smooth

### ** Examples

require(stats); require(graphics)
bs(women$height, df = 5)
summary(fm1 <- lm(weight ~ bs(height, df = 5), data = women))

## example of safe prediction
plot(women, xlab = "Height (in)", ylab = "Weight (lb)")
ht <- seq(57, 73, length.out = 200)
lines(ht, predict(fm1, data.frame(height = ht)))
## Don't show: 
## Consistency:
x <- c(1:3, 5:6)
stopifnot(identical(bs(x), bs(x, df = 3)),
          identical(bs(x, df = 4), bs(x, df = 4, knots = NULL)), # not true till 2.15.2
          !is.null(kk <- attr(bs(x), "knots")), # not true till 1.5.1
          length(kk) == 0)
## End(Don't show)


