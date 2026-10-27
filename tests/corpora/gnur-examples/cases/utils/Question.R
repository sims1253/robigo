RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Question
### Title: Documentation Shortcuts
### Aliases: Question ?
### Keywords: documentation

### ** Examples

?lapply

?"for"                  # but quotes/backticks are needed
?`+`

?women                  # information about data set "women"

package?parallel        # overview help page of package 'parallel'

## Not run: 
##D require(methods)
##D ## define a S4 generic function and some methods
##D combo <- function(x, y) c(x, y)
##D setGeneric("combo")
##D setMethod("combo", c("numeric", "numeric"), function(x, y) x+y)
##D 
##D ## assume we have written some documentation
##D ## for combo, and its methods ....
##D 
##D ?combo  # produces the function documentation
##D 
##D methods?combo  # looks for the overall methods documentation
##D 
##D method?combo("numeric", "numeric")  # documentation for the method above
##D 
##D ?combo(1:10, rnorm(10))  # ... the same method, selected according to
##D                          # the arguments (one integer, the other numeric)
##D 
##D ?combo(1:10, letters)    # documentation for the default method
## End(Not run)


