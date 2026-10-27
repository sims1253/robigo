### Name: Primitive
### Title: Look Up a Primitive Function
### Aliases: .Primitive primitive
### Keywords: interface

### ** Examples

mysqrt <- .Primitive("sqrt")
## No test: 
##D  ## be careful, this changes base R's sqrt() {until restarting R} !
##D try(structure(mysqrt, comment = "primitive function")) # deprecation warning; soon an error
## End(No test)
.Internal # this one *must* be primitive!
`if` # need backticks



