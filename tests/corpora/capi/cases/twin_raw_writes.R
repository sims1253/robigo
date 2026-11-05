dyn.load("twin_raw_writes.so")

e <- new.env()
invisible(.Call("capi_raw_init", e))
for (i in c(3L, -1L, 7L)) print(.Call("capi_raw_eval", i))
print(e$cached_call)

x <- c(1, 2, 3, 4)
invisible(.Call("capi_raw_shrink", x, 2L))
print(x)
