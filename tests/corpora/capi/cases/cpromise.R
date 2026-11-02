dyn.load("cpromise.so")

e <- .Call("capi_cpromise")
print(e$x)
print(e$y)
print(e$value)
print(e$z)
print(is.null(e$z))
print(e$w)
f <- function() e$x + 1
print(f())
print(.Call("capi_cpromise_is_unbound"))
.Call("capi_cpromise_poke")
print(e$x)
# the cons setters the SET_PR* macros expand to stay synchronized too
.Call("capi_cpromise_setcar")
print(e$x)
print(ls(e))
# the association survives gc(): the binding keeps the promise, and C's
# write through the original node still reaches it
invisible(gc())
.Call("capi_cpromise_poke")
print(e$x)
