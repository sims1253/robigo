RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: getMethod
### Title: Get or Test for the Definition of a Method
### Aliases: getMethod findMethod existsMethod selectMethod hasMethod
### Keywords: programming classes methods

### ** Examples

testFun <-  function(x)x
setGeneric("testFun")
setMethod("testFun", "numeric", function(x)x+1)

hasMethod("testFun", "numeric") # TRUE

hasMethod("testFun", "integer") #TRUE, inherited

existsMethod("testFun", "integer") #FALSE

hasMethod("testFun") # TRUE, default method

hasMethod("testFun", "ANY")

## Don't show: 
## Verify the example
stopifnot(isGeneric("testFun"),
          hasMethod("testFun", "numeric"),
          hasMethod("testFun", "integer"),
          !existsMethod("testFun", "integer"),
          hasMethod("testFun"),
          hasMethod("testFun", "ANY") )
removeGeneric("testFun")
## End(Don't show)



