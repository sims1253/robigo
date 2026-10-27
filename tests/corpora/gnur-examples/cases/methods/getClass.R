RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: getClass
### Title: Get Class Definition
### Aliases: getClass getClassDef
### Keywords: programming classes

### ** Examples

getClass("numeric") ## a built in class

cld <- getClass("thisIsAnUndefinedClass", .Force = TRUE)
cld ## a NULL prototype
## If you are really curious:
utils::str(cld)
## Whereas these generate errors:
try(getClass("thisIsAnUndefinedClass"))
try(getClassDef("thisIsAnUndefinedClass"))



