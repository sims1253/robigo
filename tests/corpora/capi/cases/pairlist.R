dyn.load("pairlist.so")

x <- .Call("capi_cons")
print(x[[1]])           # the tagged pairlist
for (e in x[-1]) print(e)

call <- quote(x + y)
print(.Call("capi_cons_mutate", call))
print(call)             # the original call is not mutated

l <- .Call("capi_langs")
print(l)
print(l[[6]])
print(eval(l[[6]]))

print(.Call("capi_convert", list(TRUE, 2L, "three")))
