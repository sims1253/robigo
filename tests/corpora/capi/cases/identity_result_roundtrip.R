dyn.load("identity.so")

# an object C created and keeps, handed back by R
for (type in 0:1) {
    r <- .Call("capi_id_make_keep", type)
    print(.Call("capi_id_is_kept", r))
    r <- .Call("capi_id_make_keep", type)
    invisible(gc())
    print(.Call("capi_id_is_kept", r))
}

# a temporary C was given and keeps, handed back by R
t <- .Call("capi_id_keep_arg", c(3, 4))
print(.Call("capi_id_is_kept", t))
t <- .Call("capi_id_keep_arg", c(3, 4))
invisible(gc())
print(.Call("capi_id_is_kept", t))
l <- .Call("capi_id_keep_arg", list(1, "a"))
invisible(gc())
print(.Call("capi_id_is_kept", l))
print(.Call("capi_id_is_kept", list(1, "a")))
