RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(splines)
### Name: asVector
### Title: Coerce an Object to a Vector
### Aliases: asVector
### Keywords: models

### ** Examples

require(stats)
ispl <- interpSpline( weight ~ height,  women )
pred <- predict(ispl)
class(pred)
utils::str(pred)
asVector(pred)



