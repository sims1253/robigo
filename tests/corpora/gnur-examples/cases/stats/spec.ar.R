RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: spec.ar
### Title: Estimate Spectral Density of a Time Series from AR Fit
### Aliases: spec.ar
### Keywords: ts

### ** Examples

require(graphics)

spec.ar(lh)

spec.ar(ldeaths)
spec.ar(ldeaths, method = "burg")

spec.ar(log(lynx))
spec.ar(log(lynx), method = "burg", add = TRUE, col = "purple")
spec.ar(log(lynx), method = "mle", add = TRUE, col = "forest green")
spec.ar(log(lynx), method = "ols", add = TRUE, col = "blue")



