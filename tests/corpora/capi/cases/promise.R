dyn.load("promise.so")

e <- new.env()
delayedAssign("d", 6 * 7, assign.env = e)
pe <- .Call("capi_promise_expr", e, as.name("d"))
print(pe)
print(identical(pe, quote(6 * 7)))
print(pe[[3]])
