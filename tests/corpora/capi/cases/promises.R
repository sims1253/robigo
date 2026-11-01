dyn.load("promises.so")

# rlang's env_clone rebuilds a namespace's lazy bindings as fresh promises
e <- .Call("capi_promise_raw", quote(6 * 7), globalenv())
print(e$v)                       # forcing evaluates the expression
f <- .Call("capi_promise_raw", quote(function(x) x + 1), baseenv())
print(f$v)
print(f$v(41))                   # a C-built function promise works as a call

g <- .Call("capi_promise_forced", list(1, 2))
print(g$v)
print(typeof(g$v))

h <- .Call("capi_promise_raw", quote(sin(0)), globalenv())
print(.Call("capi_promise_getvar_typeof", h))
print(.Call("capi_promise_getvar_typeof", f))
