RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: S4groupGeneric
### Title: S4 Group Generic Functions
### Aliases: S4groupGeneric GroupGenericFunctions Math Ops matrixOps
###   Summary Arith Logic Compare Complex Math2
### Keywords: methods

### ** Examples

setClass("testComplex", slots = c(zz = "complex"))
## method for whole group "Complex"
getGroupMembers("Complex") # "Arg"  "Conj" "Im"   "Mod"  "Re"  
setMethod("Complex", "testComplex",
          function(z) c("groupMethod", callGeneric(z@zz)))
## exception for Arg() :
setMethod("Arg", "testComplex",
          function(z) c("ArgMethod", Arg(z@zz)))
z1 <- 1+2i
z2 <- new("testComplex", zz = z1)
stopifnot(identical(Mod(z2), c("groupMethod", Mod(z1))))
stopifnot(identical(Arg(z2), c("ArgMethod", Arg(z1))))
selectMethod("Re", signature = "testComplex") # shows Generic: .. "Re" & .."Complex"
## Don't show: 
removeMethods("Complex")
removeMethods("Arg")
## End(Don't show)


