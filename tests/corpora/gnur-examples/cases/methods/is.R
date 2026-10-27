RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: is
### Title: Is an Object from a Class?
### Aliases: is extends
### Keywords: programming classes methods

### ** Examples

## Not run: 
##D ## this example can be run if package XRPython from CRAN is installed.
##D supers <- extends("PythonInterface")
##D ## find all the superclasses from package XR
##D fromXR <- sapply(supers,
##D     function(what) getClassDef(what)@package == "XR")
##D ## print them
##D supers[fromXR]
##D 
##D ## find all the superclasses at distance 2
##D superRelations <- extends("PythonInterface", fullInfo = TRUE)
##D dist2 <- sapply(superRelations,
##D     function(what) is(what, "SClassExtension") && what@distance == 2)
##D ## print them
##D names(superRelations)[dist2]
##D 
## End(Not run)



