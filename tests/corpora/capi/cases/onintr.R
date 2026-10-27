dyn.load("errors.so")  # capi_onintr ends the script like Ctrl-C

print(.Call("capi_onintr"))
print("never reached")
