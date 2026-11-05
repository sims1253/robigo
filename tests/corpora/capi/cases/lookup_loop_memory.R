dyn.load("lookup_loop.so")

rss <- function() {
    line <- grep("^VmRSS:", readLines("/proc/self/status"), value = TRUE)
    as.numeric(gsub("[^0-9]", "", line)) / 1024
}
e <- new.env()
e$v <- as.numeric(1:1000)
e$l <- list(a = 1:10, b = letters)
sym_v <- as.name("v")
sym_l <- as.name("l")
for (i in 1:1000) x <- .Call("capi_loop_get", e, sym_v)
invisible(gc())
before <- rss()
for (i in 1:100000) {
    x <- .Call("capi_loop_get", e, sym_v)
    y <- .Call("capi_loop_getvar", e, sym_l)
}
# a variable rebound between lookups: what the old values were is garbage
for (i in 1:20000) {
    e$w <- as.numeric(1:100) + i
    z <- .Call("capi_loop_get", e, as.name("w"))
}
invisible(gc())
growth <- rss() - before
print(growth < 50)
print(identical(x, e$v))
print(z[1:3])
