# The boot heap's freeze: a weak reference whose key only a boot option's
# cell holds; the cell removed, gc(): the key is dead, as in GNU R
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/weak_keys.so"))
e <- new.env()
w <- .Call("weak", e, "value", NULL)
options(dvipscmd = e)
rm(e)
invisible(gc())
print(is.null(.Call("key", w)))
options(dvipscmd = NULL)
1  # (.Last.value no longer the old option)
invisible(gc())
print(is.null(.Call("key", w)))
