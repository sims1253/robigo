dyn.load("console.so")

print(.Call("capi_printf"))
print(.Call("capi_print", matrix(1:4, 2)))
