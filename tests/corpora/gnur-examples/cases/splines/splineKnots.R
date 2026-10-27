RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(splines)
### Name: splineKnots
### Title: Knot Vector from a Spline
### Aliases: splineKnots
### Keywords: models

### ** Examples

ispl <- interpSpline( weight ~ height, women )
splineKnots( ispl )



