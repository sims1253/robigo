dyn.load("cons_tail.so")

# CDR of a cell consed onto a call is that call
x <- quote(c(a, b))
first <- .Call("capi_cons", NULL, x)
print(typeof(first))
print(length(first))
r <- .Call("capi_cdr", first)
print(typeof(r))
print(identical(r, x))
print(.Call("capi_same", r, x))
print(typeof(.Call("capi_cdr", r)))

# writing through the tail writes the pairlist it was consed onto
w <- pairlist(1, 2)
f <- .Call("capi_cons", 0, w)
invisible(.Call("capi_setcar", .Call("capi_cdr", f), 10))
print(unlist(as.list(w)))

# dropping an argument through the tail (tidyselect's c(, ))
e <- quote(c(a, , b))
head <- .Call("capi_cons", NULL, e)
cell <- .Call("capi_cdr", head)
invisible(.Call("capi_setcdr", .Call("capi_cdr", cell), .Call("capi_cdr", .Call("capi_cdr", .Call("capi_cdr", cell)))))
print(e)

# a call consed onto a call
l <- .Call("capi_lcons", quote(g), .Call("capi_cdr", quote(f(1, 2))))
print(l)
print(.Call("capi_same", .Call("capi_cdr", l), .Call("capi_cdr", l)))
