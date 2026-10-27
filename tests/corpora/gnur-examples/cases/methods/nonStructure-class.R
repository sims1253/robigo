### Name: nonStructure-class
### Title: A non-structure S4 Class for basic types
### Aliases: nonStructure-class Math,nonStructure-method
###   Math2,nonStructure-method Ops,vector,nonStructure-method
###   Ops,nonStructure,vector-method Ops,nonStructure,nonStructure-method
### Keywords: classes

### ** Examples

setClass("NumericNotStructure", contains = c("numeric","nonStructure"))
xx <- new("NumericNotStructure", 1:10)
xx + 1 # vector
log(xx) # vector
sample(xx) # vector
## Don't show: 
removeClass("NumericNotStructure")
## End(Don't show)



