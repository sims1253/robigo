### Name: setClass
### Title: Create a Class Definition
### Aliases: setClass classGeneratorFunction-class
### Keywords: programming classes methods

### ** Examples

## Don't show: 
 if(isClass("trackMultiCurve")) removeClass("trackMultiCurve")
 if(isClass("trackCurve"))      removeClass("trackCurve")
 if(isClass("track"))           removeClass("track")
## End(Don't show)
## A simple class with two slots
track <- setClass("track", slots = c(x="numeric", y="numeric"))
## an object from the class
t1 <- track(x = 1:10, y = 1:10 + rnorm(10))

## A class extending the previous, adding one more slot
trackCurve <- setClass("trackCurve",
		slots = c(smooth = "numeric"),
		contains = "track")

## an object containing a superclass object
t1s <- trackCurve(t1, smooth = 1:10)

## A class similar to "trackCurve", but with different structure
## allowing matrices for the "y" and "smooth" slots
setClass("trackMultiCurve",
         slots = c(x="numeric", y="matrix", smooth="matrix"),
         prototype = list(x=numeric(), y=matrix(0,0,0),
                          smooth= matrix(0,0,0)))

## A class that extends the built-in data type "numeric"

numWithId <- setClass("numWithId", slots = c(id = "character"),
         contains = "numeric")

numWithId(1:3, id = "An Example")

## inherit from reference object of type "environment"
stampedEnv <- setClass("stampedEnv", contains = "environment",
                       slots = c(update = "POSIXct"))
setMethod("[[<-", c("stampedEnv", "character", "missing"),
   function(x, i, j, ..., value) {
       ev <- as(x, "environment")
       ev[[i]] <- value  #update the object in the environment
       x@update <- Sys.time() # and the update time
       x})


e1 <- stampedEnv(update = Sys.time())

e1[["noise"]] <- rnorm(10)

## Don't show: 
tMC <- new("trackMultiCurve")
is.matrix(slot(tMC, "y"))
is.matrix(slot(tMC, "smooth"))
setClass("myMatrix", "matrix", prototype = matrix(0,0,0))
nrow(new("myMatrix")) # 0
nrow(new("matrix")) # 1
## simple test of prototype data
xxx <- stats::rnorm(3)
setClass("xNum", slots = c(x = "numeric"), prototype = list(x = xxx))
stopifnot(identical(new("xNum")@x, xxx))

removeClass("xNum")
removeClass("myMatrix")

## The following should not be needed.  But make check removes all files
## between example files, in a crude way that does not cause the class
## information to be reset.  There seems no way to detect this, so we
## have to remove classes ourselves

removeClass("trackMultiCurve")
removeClass("trackCurve")
removeClass("track")
## End(Don't show)



