### Name: isS4
### Title: Test for an S4 object
### Aliases: isS4 asS4 S4 asS3
### Keywords: programming

### ** Examples

## Don't show: 
require(methods)
## End(Don't show)
isS4(pi) # FALSE
isS4(getClass("MethodDefinition")) # TRUE
## Don't show: 
stopifnot(isS4(asS4(Sys.time())))
## Following is a correction of previous behavior. See note in the
## value section above
stopifnot(isS4(asS4(getClass("MethodDefinition"), FALSE, 2)))
stopifnot(!isS4(asS4(getClass("MethodDefinition"), FALSE, 0)))
## End(Don't show)



