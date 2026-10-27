RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: S3method
### Title: Register S3 Methods
### Aliases: .S3method

### ** Examples

## Create a generic function and register a method for objects
## inheriting from class 'cls':
gen <- function(x) UseMethod("gen")
met <- function(x) writeLines("Hello world.")
.S3method("gen", "cls", met)
## Create an object inheriting from class 'cls', and call the
## generic on it:
x <- structure(123, class = "cls")
gen(x)



