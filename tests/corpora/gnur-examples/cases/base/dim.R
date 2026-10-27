### Name: dim
### Title: Dimensions of an Object
### Aliases: dim dim.data.frame dim<-
### Keywords: array

### ** Examples

x <- 1:12 ; dim(x) <- c(3,4)
x

# simple versions of nrow and ncol could be defined as follows
nrow0 <- function(x) dim(x)[1]
ncol0 <- function(x) dim(x)[2]



