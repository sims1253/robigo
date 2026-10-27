RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: predict.HoltWinters
### Title: Prediction Function for Fitted Holt-Winters Models
### Aliases: predict.HoltWinters
### Keywords: ts

### ** Examples

require(graphics)

m <- HoltWinters(co2)
p <- predict(m, 50, prediction.interval = TRUE)
plot(m, p)



