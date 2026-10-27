RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(splines)
### Name: backSpline
### Title: Monotone Inverse Spline
### Aliases: backSpline
### Keywords: models

### ** Examples

require(graphics)
ispl <- interpSpline( women$height, women$weight )
bspl <- backSpline( ispl )
plot( bspl )                   # plots over the range of the knots
points( women$weight, women$height )



