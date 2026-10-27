dyn.load("errors.so")

options(warn = 1)
print(.Call("capi_warn", quote(warning_target(3))))
print(tryCatch(.Call("capi_badlongvector"),
               error = function(e) conditionMessage(e)))
print(tryCatch(.Call("capi_protect_overflow"),
               error = function(e) conditionMessage(e)))
print(tryCatch(.Call("capi_reprotect_error"),
               error = function(e) conditionMessage(e)))
print(.Call("capi_errcall", quote(the_call(1))))
print(.Call("capi_err"))
print("never reached")
