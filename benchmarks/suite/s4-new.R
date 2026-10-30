library(methods)
setClass("PerfPoint", slots = c(x = "numeric"))
x <- new("PerfPoint", x = 1)
for (i in seq_len(500)) {
    new("PerfPoint", x = 1)
}
stopifnot(identical(x@x, 1))
