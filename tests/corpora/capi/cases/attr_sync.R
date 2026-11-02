dyn.load("attr_sync.so")

# class set from R after C first saw the environment
e <- new.env()
invisible(.Call("capi_attr_touch", e))
class(e) <- "foo"
print(.Call("capi_attr_unrelated"))
print(class(e))
print(is.object(e))
print(.Call("capi_attr_read", e, "class"))
print(.Call("capi_attr_object", e))
print(class(e))

# changed and removed from R
attr(e, "tag") <- 1:3
print(.Call("capi_attr_read", e, "tag"))
attr(e, "tag") <- NULL
print(.Call("capi_attr_read", e, "tag"))
print(names(attributes(e)))
class(e) <- NULL
print(.Call("capi_attr_object", e))
print(attributes(e))

# set from C, read from R, then from C again
.Call("capi_attr_set", e, "from_c", "yes")
print(attr(e, "from_c"))
attr(e, "from_r") <- "also"
print(.Call("capi_attr_read", e, "from_c"))
print(.Call("capi_attr_read", e, "from_r"))
.Call("capi_attr_set", e, "from_c", NULL)
print(sort(names(attributes(e))))

# set by R code that C called, on an object C holds
g <- new.env()
invisible(.Call("capi_attr_touch", g))
.Call("capi_attr_callback", function(x) class(x) <- "viaR", g)
print(class(g))
print(.Call("capi_attr_object", g))

# a proxied environment that is not an argument of the routine
h <- new.env()
invisible(.Call("capi_attr_touch", h))
attr(h, "kept") <- TRUE
for (i in 1:3) invisible(.Call("capi_attr_unrelated"))
print(attr(h, "kept"))

# external pointers: made by C, classed by R
p <- .Call("capi_attr_extptr")
class(p) <- "ptr"
print(.Call("capi_attr_unrelated"))
print(class(p))
print(.Call("capi_attr_read", p, "class"))
print(.Call("capi_attr_object", p))
print(class(p))
q <- p
attr(q, "shared") <- "by reference"
print(attr(p, "shared"))
print(.Call("capi_attr_read", p, "shared"))
.Call("capi_attr_set", p, "from_c", 1L)
print(attr(q, "from_c"))
attr(p, "shared") <- NULL
print(.Call("capi_attr_read", q, "shared"))
print(sort(names(attributes(q))))

# attributes that refer to the object, on both sides
s <- new.env()
attr(s, "self") <- s
print(identical(.Call("capi_attr_read", s, "self"), s))
t1 <- new.env()
t2 <- new.env()
invisible(.Call("capi_attr_touch", t1))
attr(t1, "other") <- t2
attr(t2, "other") <- t1
print(identical(.Call("capi_attr_read", t1, "other"), t2))
print(identical(.Call("capi_attr_read", t2, "other"), t1))

# a replacement equal to the old value is still the routine's object
dyn.load("pairlist.so")
old <- pairlist(1, 2)
new <- pairlist(1, 2)
r <- new.env()
attr(r, "a") <- old
invisible(.Call("capi_attr_touch", r))
.Call("capi_attr_set", r, "a", new)
.Call("capi_setcar_alias", new)
print(attr(r, "a"))
