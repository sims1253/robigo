## help-page examples 'proto-package', from the pure-R CRAN package proto 1.0.0 (GPL-2).
## Run with the package attached, as R CMD check runs help-page examples.
.libPaths(c("lib", .libPaths()))
suppressPackageStartupMessages(library(proto))
set.seed(1)
### Name: proto-package
### Title: Object-Oriented Programming with the Prototype Model
### Aliases: proto-package
### Keywords: programming

### ** Examples


cat("parent\n")
oop <- proto(x = 10, view = function(.) paste("this is a:", .$x))
oop$ls()
oop$view()

cat("override view in parent\n")
ooc1 <- oop$proto(view = function(.) paste("this is a: ***", .$x, "***"))
ooc1$view()

cat("override x in parent\n")
ooc2 <- oop$proto(x = 20)
ooc2$view()



