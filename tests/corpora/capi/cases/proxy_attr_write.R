dyn.load("proxy_attr_write.so")

f <- function(x) x
e <- new.env()
show <- function() {
    print(attributes(f))
    print(attributes(e))
    print(c(isS4(f), isS4(e)))
}
invisible(.Call("capi_aw_keep", f, e))

invisible(.Call("capi_aw_set"))
show()
invisible(.Call("capi_aw_setcar"))
show()
invisible(.Call("capi_aw_append"))
show()
invisible(.Call("capi_aw_retag"))
print(names(attributes(e)))

# a cell kept from one call, written in another, with calls between
invisible(.Call("capi_aw_keep_cell"))
for (i in 1:3) invisible(.Call("capi_aw_nothing"))
invisible(.Call("capi_aw_poke_kept"))
print(attr(e, "a"))

invisible(.Call("capi_aw_s4", TRUE))
show()
invisible(.Call("capi_aw_s4", FALSE))
show()

# R code the routine runs sees what it wrote before
print(.Call("capi_aw_then_eval", function() attr(e, "a")))
print(attr(e, "a"))
