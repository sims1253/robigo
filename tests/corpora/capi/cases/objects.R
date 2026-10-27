dyn.load("objects.so")

Point <- methods::setClass("Point", slots = c(x = "numeric", y = "numeric"))
p <- methods::new("Point", x = 1, y = 2)
print(.Call("capi_slots", p, as.name("x"), 41))

q <- .Call("capi_new_object")
print(q[[1]])
print(q[[2]]@x)
print(q[[2]]@y)

f <- .Call("capi_closure")
print(f[[1]])
print(f[[2]])
print(f[[3]])
print(f[[4]])
print(f[[5]])

e <- new.env()
delayedAssign("d", 6 * 7, assign.env = e)
pr <- .Call("capi_promise_parts", e, as.name("d"))
print(pr[[1]])
print(pr[[3]])
print(e$d)

print(.Call("capi_exprs", function(x) x + 1))
