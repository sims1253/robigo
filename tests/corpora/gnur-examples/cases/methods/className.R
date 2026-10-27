RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: className
### Title: Class names including the corresponding package
### Aliases: className multipleClasses className-class
### Keywords: classes programming

### ** Examples

## Not run: 
##D className("vector") # will be found, from package "methods"
##D className("vector", "magic") # OK, even though the class doesn't exist
##D 
##D 
##D className("An unknown class") # Will cause an error
## End(Not run)



