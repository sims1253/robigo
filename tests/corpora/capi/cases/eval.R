dyn.load("eval.so")

print(.Call("capi_eval"))
print(.Call("capi_tryeval"))
print(.Call("capi_parse"))
print(.Call("capi_option"))
