RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: predict.loess
### Title: Predict LOESS Curve or Surface
### Aliases: predict.loess
### Keywords: smooth

### ** Examples

cars.lo <- loess(dist ~ speed, cars)
predict(cars.lo, data.frame(speed = seq(5, 30, 1)), se = TRUE)
# to get extrapolation
cars.lo2 <- loess(dist ~ speed, cars,
  control = loess.control(surface = "direct"))
predict(cars.lo2, data.frame(speed = seq(5, 30, 1)), se = TRUE)



