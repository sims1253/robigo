RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: setGeneric
### Title: Create a Generic Version of a Function
### Aliases: setGeneric
### Keywords: programming methods

### ** Examples


## Specify that this package will define methods for plot()
setGeneric("plot")

## create a new generic function, with a default method
setGeneric("props", function(object) attributes(object))

###   A non-standard generic function.  It insists that the methods
###   return a non-empty character vector (a stronger requirement than
###    valueClass = "character" in the call to setGeneric)

setGeneric("authorNames",
    function(text) {
      value <- standardGeneric("authorNames")
      if(!(is(value, "character") && any(nchar(value)>0)))
        stop("authorNames methods must return non-empty strings")
      value
      })

## the asRObject generic function, from package XR
## Its default method just returns object
## See the reference, Chapter 12 for methods

setGeneric("asRObject", function(object, evaluator) {
        object
})


## Don't show: 
setMethod("authorNames", "character", function(text)text)

tryIt <- function(expr) tryCatch(expr, error = function(e) e)
stopifnot(identical(authorNames(c("me", "you")), c("me", "you")),
          is(tryIt(authorNames(character())), "error"), # empty value
          is(tryIt(authorNames(NULL)), "error"))        # no default method
## End(Don't show)
## Don't show: 
removeGeneric("authorNames")
removeGeneric("props")
removeGeneric("asRObject")
## End(Don't show)



