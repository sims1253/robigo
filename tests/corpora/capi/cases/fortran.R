dyn.load("fortran.so")

print(.Call("capi_qr"), digits = 12)
print(.Call("capi_optim"), digits = 12)
.Call("capi_intpr")
print(.Call("capi_dqags"), digits = 15)
