RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: attr
### Title: Object Attributes
### Aliases: attr attr<-
### Keywords: attribute

### ** Examples

# create a 2 by 5 matrix
x <- 1:10
attr(x,"dim") <- c(2, 5)

S <- sum
attr(S, "foo") <- NA   # gives a warning, will become an error!
attributes(sum) <- NULL # revert to sanity



