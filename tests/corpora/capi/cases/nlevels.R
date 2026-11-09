dyn.load("nlevels.so")

# a factor: the length of its levels attribute
print(.Call("capi_nlevels", factor(c("b", "a", "b"))))
print(.Call("capi_nlevels", factor(character())))
print(.Call("capi_nlevels", factor(1:3, levels = 1:5)))
print(.Call("capi_nlevels", ordered(c("lo", "hi"), levels = c("lo", "mid", "hi"))))
# an integer with class factor and no levels: 0
print(.Call("capi_nlevels", structure(1L, class = "factor")))
# not a factor: 0, whatever its levels attribute
print(.Call("capi_nlevels", structure(1:2, levels = c("a", "b"))))
print(.Call("capi_nlevels", 1:3))
print(.Call("capi_nlevels", NULL))
# structure() stores a double factor's codes as integers: a factor
print(.Call("capi_nlevels", structure(1, class = "factor", levels = "a")))
