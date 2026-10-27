dyn.load("math.so")

set.seed(42)
print(.Call("capi_rng"), digits = 15)
print(.Call("capi_rdist"), digits = 15)
print(.Call("capi_math"), digits = 15)
print(.Call("capi_dpq"), digits = 15)
set.seed(42)
print(.Call("capi_qr_more"), digits = 15)
