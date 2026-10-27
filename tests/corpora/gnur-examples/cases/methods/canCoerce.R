RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: canCoerce
### Title: Can an Object be Coerced to a Certain S4 Class?
### Aliases: canCoerce
### Keywords: classes methods

### ** Examples

m <- matrix(pi, 2,3)
canCoerce(m, "numeric") # TRUE
canCoerce(m, "array")   # TRUE



