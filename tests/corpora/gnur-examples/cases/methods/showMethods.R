RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: showMethods
### Title: Show all the methods for the specified function(s) or class
### Aliases: showMethods .S4methods
### Keywords: methods

### ** Examples

require(graphics)
## Don't show: 
 setClass("track", slots = c(x="numeric", y="numeric"))
 ## First, with only one object as argument:
 setMethod("plot", signature(x="track", y="missing"),
           function(x,  y, ...) plot(slot(x, "x"), slot(x, "y"), ...))
 ## Second, plot the data from the track on the y-axis against anything
 ## as the x data.
 setMethod("plot", signature(y = "track"),
           function(x, y, ...) plot(x, slot(y, "y"), ...))
 setMethod("plot", "track",
           function(x, y, ...) plot(slot(x, "y"), y,  ...))
## End(Don't show)
## Assuming the methods for plot
## are set up as in the example of help(setMethod),
## print (without definitions) the methods that involve class "track":
showMethods("plot", classes = "track")
## Not run: 
##D # Function "plot":
##D # x = ANY, y = track
##D # x = track, y = missing
##D # x = track, y = ANY
##D 
##D require("Matrix")
##D showMethods("%*%")# many!
##D     methods(class = "Matrix")# nothing
##D showMethods(class = "Matrix")# everything
##D showMethods(Matrix:::isDiagonal) # a non-exported generic
## End(Not run)

if(no4 <- is.na(match("stats4", loadedNamespaces())))
   loadNamespace("stats4")
showMethods(classes = "mle") # -> a method for show()
if(no4) unloadNamespace("stats4")



