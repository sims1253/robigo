# cbind/rbind with S4 arguments: methods:::cbind/rbind and cbind2/rbind2 methods
library(methods)
setClass("myMat", representation(x = "numeric"))
setMethod("cbind2", signature(x = "myMat", y = "missing"), function(x, y, ...) x)
m <- new("myMat", x = c(1, pi))
stopifnot(identical(m, methods:::cbind(m)), identical(m, cbind(m)))
setMethod("rbind2", signature(x = "myMat", y = "missing"), function(x, y, ...) x)
identical(rbind(m), m)
setClass("Num2", contains = "numeric")
n <- new("Num2", 1:3)
cbind(n)
cbind(1:2, 3:4)
rbind.myS3 <- function(..., deparse.level = 1) "rbind.myS3"
rbind(structure(1, class = "myS3"), m)
