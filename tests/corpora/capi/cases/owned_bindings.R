dyn.load("owned_bindings.so")

e <- .Call("capi_owned_env")
print(e$v)
invisible(.Call("capi_owned_bump"))  # C changes its object in place
print(e$v)
print(bindingIsActive("v", e))
print(tryCatch(activeBindingFunction("v", e), error = conditionMessage))
print(tryCatch(makeActiveBinding("v", function() 1, e), error = conditionMessage))
print(as.list(e))
print(eapply(e, sum))
print(mget("v", envir = e))

# serialization writes the value as a plain binding
x <- unserialize(serialize(e, NULL))
print(x$v)
print(bindingIsActive("v", x))
f <- tempfile()
saveRDS(e, f)
y <- readRDS(f)
print(y$v)
print(isTRUE(all.equal(e, y)))
print(identical(as.list(e), as.list(x)))

# an R assignment replaces the value; C's object is no longer bound
e$v <- 5
invisible(.Call("capi_owned_bump"))
print(e$v)

# a real active binding: as.list reads it, serialize keeps it active
a <- new.env()
makeActiveBinding("act", function() 42, a)
print(as.list(a))
z <- unserialize(serialize(a, NULL))
print(bindingIsActive("act", z))
print(z$act)
