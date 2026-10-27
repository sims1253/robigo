RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: getPackageName
### Title: The Name associated with a Given Package
### Aliases: getPackageName setPackageName packageSlot packageSlot<-
### Keywords: programming

### ** Examples

## all the following usually return "base"
getPackageName(length(search()))
getPackageName(baseenv())
getPackageName(asNamespace("base"))
getPackageName("package:base")




