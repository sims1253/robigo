RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: setOldClass
### Title: Register Old-Style (S3) Classes and Inheritance
### Aliases: setOldClass .setOldIs POSIXct-class POSIXlt-class POSIXt-class
###   aov-class maov-class anova-class anova.glm-class anova.glm.null-class
###   Date-class data.frame-class data.frameRowLabels-class density-class
###   dump.frames-class factor-class formula-class glm-class glm.null-class
###   hsearch-class integrate-class libraryIQR-class lm-class logLik-class
###   mlm-class mtable-class mts-class ordered-class packageIQR-class
###   packageInfo-class recordedplot-class rle-class socket-class
###   summaryDefault-class summary.table-class oldClass-class
###   .OldClassesList table-class initialize,data.frame-method
###   initialize,factor-method initialize,ordered-method
###   initialize,table-method initialize,summary.table-method
### Keywords: programming methods

### ** Examples

## Don't show: 
## All the predefined S3 classes with S4 versions
sort(unlist(.OldClassesList))
## End(Don't show)

require(stats)

## "lm" and "mlm" are predefined; if they were not this would do it:
## Not run: 
##D setOldClass(c("mlm", "lm"))
## End(Not run)

## Define a new generic function to compute the residual degrees of freedom
setGeneric("dfResidual",
  function(model) stop(gettextf(
    "This function only works for fitted model objects, not class %s",
                                class(model))))

setMethod("dfResidual", "lm", function(model)model$df.residual)

## dfResidual will work on mlm objects as well as lm objects
myData <- data.frame(time = 1:10, y = (1:10)^.5)
myLm <- lm(cbind(y, y^3)  ~ time, myData)

## Don't show: 
showClass("data.frame")# to see the predefined S4 "oldClass"
## End(Don't show)

## two examples extending S3 class "lm": class "xlm" directly
## and "ylm" indirectly
setClass("xlm", slots = c(eps = "numeric"), contains = "lm")
setClass("ylm", slots = c(header = "character"), contains = "xlm")
ym1 = new("ylm", myLm, header = "Example", eps = 0.)
## for more examples, see ?S3Class.


## Don't show: 
stopifnot(identical(dfResidual(myLm), myLm$df.residual))
removeClass("ylm"); removeClass("xlm")
rm(myData, myLm)
removeGeneric("dfResidual")
## End(Don't show)

## Not run: 
##D ## The code in R that defines "ts" as an S4 class
##D setClass("ts", contains = "structure", slots = c(tsp = "numeric"),
##D          prototype(NA, tsp = rep(1,3)))
##D        # prototype to be a legal S3 time-series
##D ## and now registers it as an S3 class
##D setOldClass("ts", S4Class = "ts", where = envir)
## End(Not run)




