dyn.load("cpromise_altrep.so")

e <- .Call("capi_cpromise_altrep")
print(e$v[1L])
# forced by the evaluator, outside any .Call
print(e$x)
print(e$y)
f <- function() e$x * 2L
print(f())
print(.Call("capi_cpromise_altrep_value"))
