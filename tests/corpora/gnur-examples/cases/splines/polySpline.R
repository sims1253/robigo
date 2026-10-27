RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(splines)
### Name: polySpline
### Title: Piecewise Polynomial Spline Representation
### Aliases: polySpline as.polySpline
### Keywords: models

### ** Examples

require(graphics)
ispl <- polySpline(interpSpline( weight ~ height,  women, bSpline = TRUE))
## IGNORE_RDIFF_BEGIN
print( ispl )   # print the piecewise polynomial representation
## IGNORE_RDIFF_END
plot( ispl )    # plots over the range of the knots
points( women$height, women$weight )



