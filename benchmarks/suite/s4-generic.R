library(methods)
setClass("PerfPoint", slots = c(x = "numeric"))
x <- new("PerfPoint", x = 1)
setGeneric("coordinate", function(x) standardGeneric("coordinate"))
setMethod("coordinate", "PerfPoint", function(x) x@x)
for (i in seq_len(10000)) {
    coordinate(x)
}
stopifnot(identical(x@x, 1))
