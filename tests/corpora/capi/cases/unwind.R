dyn.load("unwind.so")

print(tryCatch(.Call("capi_unwind"),
               error = function(e) paste("caught:", conditionMessage(e))))
print(.Call("capi_unwind_ok"))
print(.Call("capi_execcleanup"))
print(.Call("capi_toplevelexec"))
