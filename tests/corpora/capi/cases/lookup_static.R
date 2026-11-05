dyn.load("lookup_static.so")

e <- new.env()
assign("marker", structure(list(), class = "marker"), envir = e)
assign("pair", c(a = 1.5, b = 2.5), envir = e)
invisible(.Call("capi_lookup_init", e))
for (i in 1:3) {
    invisible(gc())
    invisible(.Call("capi_lookup_churn", 5000L))
    print(.Call("capi_lookup_cached"))
    print(.Call("capi_lookup_is", e$marker, e$pair))
}
