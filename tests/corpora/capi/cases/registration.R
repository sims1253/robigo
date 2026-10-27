dyn.load("registration.so")  # runs R_init_registration

print(.Call("c_add", 20L, 22L))
print(.C("c_sum2", as.integer(1:4), 4L, out = integer(1L))$out)
print(.External("c_first", "ext-first", 2))
print(.External("c_count_args", "n", 1, 2, 3))

dyn.load("registration2.so")  # forces native symbols
print(try(.Call("c_mul", 6L, 7L), silent = TRUE))  # by name: refused
dll <- getLoadedDLLs()[["registration2"]]
mul <- getNativeSymbolInfo("c_mul", PACKAGE = dll)
print(.Call(mul, 6L, 7L))
