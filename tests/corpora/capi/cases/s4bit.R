dyn.load("s4bit.so")

f <- asS4(structure(function(x) x + 1, class = "fun"))
print(isS4(f))
print(.Call("capi_is_s4", f))
print(isS4(f))
g <- .Call("capi_identity", f)
print(c(isS4(f), isS4(g)))
print(g(1))

e <- new.env()
assign("f", f, envir = e)
h <- .Call("capi_get_var", as.name("f"), e)
print(c(isS4(h), isS4(e$f), isS4(f)))

l <- list(f, 1)
print(.Call("capi_is_s4", l[[1]]))
print(isS4(.Call("capi_identity", l)[[1]]))

env <- asS4(structure(new.env(), class = "envS4"))
print(.Call("capi_is_s4", env))
print(isS4(.Call("capi_identity", env)))
print(isS4(env))

cl <- asS4(structure(quote(f(x)), class = "callS4"))
print(.Call("capi_is_s4", cl))
print(isS4(.Call("capi_identity", cl)))
print(isS4(cl))

pl <- asS4(structure(pairlist(a = 1), class = "plS4"))
print(.Call("capi_is_s4", pl))
print(isS4(.Call("capi_identity", pl)))
print(isS4(pl))

plain <- function(x) x
print(c(.Call("capi_is_s4", plain), isS4(plain), isS4(.Call("capi_identity", plain))))
