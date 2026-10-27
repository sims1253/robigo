dyn.load("attributes.so")

m <- matrix(1:6, nrow = 2, dimnames = list(a = c("r1", "r2"), b = c("c1", "c2", "c3")))
print(.Call("capi_attrib_get", m))
print(.Call("capi_attrib_get", c(x = 1, y = 2)))

x <- 1:6
print(.Call("capi_attrib_set", x, c("p", "q", "r", "s", "t", "u")))

y <- structure(1:4, class = "capi_cls", comment = "kept by copyMostAttrib")
print(.Call("capi_attrib_list", y))
print(.Call("capi_inherits", data.frame(a = 1:3)))
print(.Call("capi_inherits", y))
print(.Call("capi_duplicate", y))
print(.Call("capi_dimnames", matrix(1:4, 2)))
print(tryCatch(.Call("capi_col2name"),
               error = function(e) paste("col2name:", conditionMessage(e))))
