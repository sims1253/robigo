RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: setAs
### Title: Methods for Coercing an Object to a Class
### Aliases: coerce coerce<- setAs coerce-methods coerce,ANY,array-method
###   coerce,ANY,call-method coerce,ANY,character-method
###   coerce,ANY,complex-method coerce,ANY,environment-method
###   coerce,ANY,expression-method coerce,ANY,function-method
###   coerce,ANY,integer-method coerce,ANY,list-method
###   coerce,ANY,logical-method coerce,ANY,matrix-method
###   coerce,ANY,name-method coerce,ANY,numeric-method
###   coerce,ANY,raw-method coerce,ANY,single-method coerce,ANY,ts-method
###   coerce,ANY,vector-method coerce,ANY,NULL-method
### Keywords: programming classes methods

### ** Examples

## using the definition of class "track" from setClass

## Don't show: 
setClass("track", slots = c(x="numeric", y="numeric"))
setClass("trackCurve", contains = "track", slots = c(smooth = "numeric"))
## End(Don't show)

setAs("track", "numeric", function(from) from@y)

t1 <- new("track", x=1:20, y=(1:20)^2)

as(t1, "numeric")

## The next example shows:
##  1. A virtual class to define setAs for several classes at once.
##  2. as() using inherited information

setClass("ca", slots = c(a = "character", id = "numeric"))

setClass("cb", slots = c(b = "character", id = "numeric"))

setClass("id")
setIs("ca", "id")
setIs("cb", "id")


setAs("id", "numeric", function(from) from@id)

CA <- new("ca", a = "A", id = 1)
CB <- new("cb", b = "B", id = 2)

setAs("cb", "ca", function(from, to )new(to, a=from@b, id = from@id))

as(CB, "numeric")

## Don't show: 
## error (should have been a function of one argument)
tools::assertError(setAs("track", "numeric", function(x, y,z)x@y), verbose=TRUE)
stopifnot(identical(raw(1L), as(0L, "raw"))) # method not defined until R 4.5
## End(Don't show)



