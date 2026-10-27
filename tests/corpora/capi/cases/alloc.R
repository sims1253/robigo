dyn.load("alloc.so")

x <- .Call("capi_alloc_types")
print(sapply(x, typeof))
print(lengths(x))

print(.Call("capi_lengths", c(1.5, 2.5)))  # length+xlength+LENGTH+XLENGTH
print(.Call("capi_lengths", NULL))

print(.Call("capi_scalars"))
print(.Call("capi_protect"))               # reprotected slot holds the last value
print(.Call("capi_unprotect_ptr"))
print(.Call("capi_preserve"))
print(.Call("capi_valloc"))                # S_realloc zeroes the tail
print(.Call("capi_alloc_more"))
