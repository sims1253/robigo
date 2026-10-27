### Name: HoltWinters
### Title: Holt-Winters Filtering
### Aliases: HoltWinters print.HoltWinters residuals.HoltWinters
### Keywords: ts

### ** Examples

## Don't show: 
od <- options(digits = 5)
## End(Don't show)
require(graphics)

## Seasonal Holt-Winters
(m <- HoltWinters(co2))
plot(m)
plot(fitted(m))

(m <- HoltWinters(AirPassengers, seasonal = "mult"))
plot(m)

## Non-Seasonal Holt-Winters
x <- uspop + rnorm(uspop, sd = 5)
m <- HoltWinters(x, gamma = FALSE)
plot(m)

## Exponential Smoothing
m2 <- HoltWinters(x, gamma = FALSE, beta = FALSE)
lines(fitted(m2)[,1], col = 3)
## Don't show: 
options(od)
## End(Don't show)



