dyn.load("proxy_statics.so")

e <- new.env()
f <- function(x) x
p <- new.env()
invisible(.Call("capi_static_keep", e, f, p))

# written by later calls that have no arguments
invisible(.Call("capi_static_classify"))
print(class(e))
print(attributes(f))
print(attr(p, "size"))

invisible(.Call("capi_static_poke"))
print(class(e))
print(attributes(f))

invisible(.Call("capi_static_replace"))
print(attributes(p))
print(isS4(e))

invisible(.Call("capi_static_clear"))
print(attributes(e))
print(isS4(e))
print(attributes(f))

# many environments C has seen, alive in R, and many calls that touch none
# of them: each call costs the same, however many there are
envs <- lapply(1:20000, function(i) new.env())
for (x in envs) invisible(.Call("capi_static_nothing", x))
for (i in 1:100000) invisible(.Call("capi_static_nothing", NULL))
print(length(envs))
