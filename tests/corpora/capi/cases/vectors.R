dyn.load("vectors.so")

x <- c(1L, 2L)
print(.Call("capi_int", x))
print(x)  # INTEGER writes are visible in the caller's fresh vector

print(.Call("capi_real", c(1.25, 2.5)))
print(.Call("capi_lgl_raw"))
print(.Call("capi_complex", c(1 + 2i, 30 + 40i)))
print(.Call("capi_na", c(NA, NaN, Inf, -Inf, 0)))
print(.Call("capi_lists", list(c(2, 4, 6), "unused"), c("hi", "there")))
print(.Call("capi_accessors", c(1 + 2i, 3 - 4i), c(TRUE, NA), as.raw(1:4), 1:6, c(1.5, 2.5)))
print(.Call("capi_setters", c(1.5, 2.5, 3.5)))
print(.Call("capi_identical"))
print(.Call("capi_sharing"))
