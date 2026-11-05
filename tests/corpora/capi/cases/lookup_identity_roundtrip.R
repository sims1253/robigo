dyn.load("lookup_identity.so")

ns <- new.env()
assign("zap", structure(list(), class = "zap"), envir = ns)
invisible(.Call("capi_rt_init", ns))
zap <- function() get("zap", envir = ns)
invisible(gc())
print(.Call("capi_rt_is", zap()))
# a list C builds around it, back through R
w <- .Call("capi_rt_wrap", zap())
print(.Call("capi_rt_first_is", w))
print(.Call("capi_rt_is", w[[1]]))
# the static itself, back through R
z <- .Call("capi_rt_get")
print(.Call("capi_rt_is", z))
print(.Call("capi_rt_first_is", list(z)))
print(identical(z, zap()))
