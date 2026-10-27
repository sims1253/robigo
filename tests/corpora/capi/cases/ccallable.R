dyn.load("ccallable_reg.so")  # registers the callables
print(.Call("capi_self"))

dyn.load("ccallable.so")      # fetches them
print(.Call("capi_use_ccallable"))
print(.Call("capi_use_gcd"))
