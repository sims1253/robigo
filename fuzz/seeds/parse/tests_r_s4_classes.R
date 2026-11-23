# setClass, new, slots, validity, show, inheritance from a basic type
library(methods)
setClass("P", representation(x = "numeric", y = "character"))
p <- new("P", x = 1, y = "a")
isS4(p)
p
p@x <- 3
p@x
slot(p, "y")
slot(p, "y") <- "b"
p@y
isVirtualClass("P")
is(p, "P")
existsMethod("show", "P")
setValidity("P", function(object) if (length(object@x) != 1) "x must have length 1" else TRUE)
r <- tryCatch(new("P", x = 1:2, y = "a"), error = function(e) conditionMessage(e))
r
setClass("N", contains = "numeric", representation(unit = "character"))
n <- new("N", 1:3, unit = "m")
n
n@.Data
n + 1
is(n, "numeric")
length(n)
setClass("Q", contains = "P", representation(z = "logical"))
q <- new("Q", x = 2, y = "c", z = TRUE)
is(q, "P")
q@x
extends("Q", "P")
slotNames("Q")
initialize(p, x = 10)@x
