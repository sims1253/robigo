dyn.load("coerce.so")

print(.Call("capi_coerce", c(TRUE, FALSE)))
print(.Call("capi_as", 2L))
print(.Call("capi_as", "3.5"))
print(.Call("capi_as", NA))
print(.Call("capi_asbool", 2L))
print(.Call("capi_asbool", "TRUE"))
print(.Call("capi_type2"))

vals <- list(NULL, 1:2, c(1.5, 2), c(TRUE), "s", list(a = 1),
             quote(sym), globalenv(), sin, factor("b"),
             matrix(1:4, 2), data.frame(a = 1),
             structure(1, class = "c"), quote(1 + 2))
p <- matrix(.Call("capi_predicates", vals), ncol = 20, byrow = TRUE)
colnames(p) <- c("null", "integer", "real", "logical", "string", "vector",
                 "atomic", "vecList", "newList", "list", "symbol", "envir",
                 "function", "numeric", "factor", "matrix", "array",
                 "dataframe", "object", "s4")
print(p)

print(.Call("capi_dims", matrix(1:6, 3)))
