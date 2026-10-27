RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Box.test
### Title: Box-Pierce and Ljung-Box Tests
### Aliases: Box.test
### Keywords: ts

### ** Examples

x <- rnorm (100)
Box.test (x, lag = 1)
Box.test (x, lag = 1, type = "Ljung")



