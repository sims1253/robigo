dyn.load("statics.so")

twice <- function(x) 2 * x
e <- new.env()
invisible(.Call("capi_statics_init", e))
for (i in 1:3) {
    invisible(gc())
    junk <- lapply(1:5000, function(i) numeric(20))
    print(.Call("capi_statics_use"))
}
