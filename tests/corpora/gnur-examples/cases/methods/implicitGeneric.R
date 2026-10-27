### Name: implicitGeneric
### Title: Manage Implicit Versions of Generic Functions
### Aliases: implicitGeneric setGenericImplicit prohibitGeneric
###   registerImplicitGenerics 'implicit generic'
### Keywords: programming methods

### ** Examples


### How we would make the function with() into a generic:

## Since the second argument, 'expr' is used literally, we want
## with() to only have "data" in the signature.

## Not run: 
##D setGeneric("with", signature = "data")
##D ## Now we could predefine methods for "with" if we wanted to.
##D 
##D ## When ready, we store the generic as implicit, and restore the
##D original
##D 
##D setGenericImplicit("with")
## End(Not run)

implicitGeneric("with")

# (This implicit generic is stored in the 'methods' package.)



