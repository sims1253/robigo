RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: is.function
### Title: Is an Object of Type (Primitive) Function?
### Aliases: is.function is.primitive
### Keywords: programming

### ** Examples

is.function(1) # FALSE
is.function (is.primitive) # TRUE: it is a function, but ..
is.primitive(is.primitive) # FALSE: it's not a primitive one, whereas
is.primitive(is.function)  # TRUE: that one *is*



