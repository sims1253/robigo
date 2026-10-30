library(methods)
setClass("PerfPoint", slots = c(x = "numeric"))
x <- new("PerfPoint", x = 1)
for (i in seq_len(10000)) {
    x@x
}
stopifnot(identical(x@x, 1))
