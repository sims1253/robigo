dyn.load("extptr.so")

print(.Call("capi_extptr"))

p <- .Call("capi_make_finalized")
print(identical(p, p))
rm(p)
invisible(gc())  # runs the finalizer

key <- new.env()
val <- "value"
w <- .Call("capi_weakref", key, val)
print(w)
