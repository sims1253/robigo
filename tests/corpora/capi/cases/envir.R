dyn.load("envir.so")

print(.Call("capi_envir", 41 + 1))
e <- new.env(parent = globalenv())
e$known <- "value"
print(.Call("capi_find", e, as.name("known")))
r <- .Call("capi_find", e, as.name("pi"))  # found through to base
print(r[c(1, 3)])
print(typeof(r[[2]]))  # lazily bound: a promise
print(.Call("capi_findfun", e))

x <- .Call("capi_envirs")
print(x[1:8])
print(length(x[[9]]) > 0)

a <- .Call("capi_active", function() 99)
print(a[1:3])
print(a[[4]]$active_one)  # reading runs the active binding

f <- function(...) .Call("capi_dots", environment())
print(f(1, , 3))
print(f())

e2 <- new.env(parent = globalenv())
print(.Call("capi_envir_more", e2))
print(.Call("capi_symbols"))

ns <- .Call("capi_namespace")
print(ns[[1]])
print(length(ns[[2]]) > 100)
