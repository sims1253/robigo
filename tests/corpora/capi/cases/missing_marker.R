dyn.load("missing_marker.so")

# R_MissingArg is the symbol "", bound to itself in base (its SYMVALUE):
# a lookup that reaches base finds the missing argument, one through a
# chain ending at emptyenv() does not.
print(.Call("capi_missing_marker", globalenv()))
print(.Call("capi_missing_marker", new.env(parent = emptyenv())))
print(tryCatch(.Call("capi_eval_missing_marker", globalenv()), error = function(e) class(e)))
print(tryCatch(.Call("capi_eval_missing_marker", new.env(parent = emptyenv())), error = conditionMessage))
print(tryCatch(eval(quote(expr = ), new.env(parent = emptyenv())), error = conditionMessage))
print(tryCatch(eval(quote(expr = )), error = conditionMessage))
print(bindingIsActive("", baseenv()))
print(tryCatch(bindingIsActive("", globalenv()), error = conditionMessage))
