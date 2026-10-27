RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: new
### Title: Generate an Object from a Class
### Aliases: new initialize
### Keywords: programming classes

### ** Examples

## using the definition of class "track" from setClass

## Don't show: 
setClass("track", slots = c(x="numeric", y="numeric"))
setClass("trackCurve", contains = "track",
          slots = c(smooth = "numeric"))

ydata <- stats::rnorm(10); ysmooth <- 1:10
## End(Don't show)

## a new object with two slots specified
t1 <- new("track", x = seq_along(ydata), y = ydata)

# a new object including an object from a superclass, plus a slot
t2 <- new("trackCurve", t1, smooth = ysmooth)

### define a method for initialize, to ensure that new objects have
### equal-length x and y slots.  In this version, the slots must still be
### supplied by name.

setMethod("initialize", "track", 
    function(.Object, ...) {
      .Object <- callNextMethod()
      if(length(.Object@x) != length(.Object@y))
      stop("specified x and y of different lengths")
      .Object
    })

### An alternative version that allows x and y to be supplied
### unnamed.  A still more friendly version would make the default x
### a vector of the same length as y, and vice versa.

setMethod("initialize", "track",
          function(.Object, x = numeric(0), y = numeric(0), ...) {
              .Object <- callNextMethod(.Object, ...)
              if(length(x) != length(y))
                  stop("specified x and y of different lengths")
              .Object@x <- x
              .Object@y <- y
              .Object
          })

## Don't show: 
removeMethod("initialize", "track")
## End(Don't show)



