RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: predict.Arima
### Title: Forecast from ARIMA fits
### Aliases: predict.Arima
### Keywords: ts

### ** Examples

od <- options(digits = 5) # avoid too much spurious accuracy
predict(arima(lh, order = c(3,0,0)), n.ahead = 12)

(fit <- arima(USAccDeaths, order = c(0,1,1),
              seasonal = list(order = c(0,1,1))))
predict(fit, n.ahead = 6)
options(od)



