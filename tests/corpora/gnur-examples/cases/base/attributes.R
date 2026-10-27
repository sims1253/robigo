RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: attributes
### Title: Object Attribute Lists
### Aliases: attributes attributes<- mostattributes<-
### Keywords: attribute

### ** Examples

x <- cbind(a = 1:3, pi = pi) # simple matrix with dimnames
attributes(x)

## strip an object's attributes:
attributes(x) <- NULL
x # now just a vector of length 6

mostattributes(x) <- list(mycomment = "really special", dim = 3:2,
   dimnames = list(LETTERS[1:3], letters[1:5]), names = paste(1:6))
x # dim(), but not {dim}names



