RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(splines)
### Name: ns
### Title: Generate a Basis Matrix for Natural Cubic Splines
### Aliases: ns
### Keywords: smooth

### ** Examples

require(stats); require(graphics)
ns(women$height, df = 5)
summary(fm1 <- lm(weight ~ ns(height, df = 5), data = women))

## To see what knots were selected
attr(terms(fm1), "predvars")

## example of safe prediction
plot(women, xlab = "Height (in)", ylab = "Weight (lb)")
ht <- seq(57, 73, length.out = 200) ; nD <- data.frame(height = ht)
lines(ht, p1 <- predict(fm1, nD))
stopifnot(all.equal(p1, predict(update(fm1, . ~
                            splines::ns(height, df=5)), nD)))
          # not true in R < 3.5.0
## Don't show: 
## Consistency:
x <- c(1:3, 5:6)
stopifnot(identical(ns(x), ns(x, df = 1)),
          identical(ns(x, df = 2),
                    ns(x, df = 2, knots = NULL)), # not true till 2.15.2
          !is.null(kk <- attr(ns(x), "knots")), # not true till 1.5.1
          length(kk) == 0)
## End(Don't show)



