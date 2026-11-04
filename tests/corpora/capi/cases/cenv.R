dyn.load("cenv.so")

# A namespace-shaped environment: `.__NAMESPACE__.` holds the spec.
info <- new.env()
info$spec <- c(name = "fakepkg", version = "1.0")
ns <- new.env()
assign(".__NAMESPACE__.", info, envir = ns)
assign(".packageName", "fakepkg", envir = ns)
assign("x", c(1, 2, 3), envir = ns)
assign("f", function() x, envir = ns)
print(isNamespace(ns))

cl <- .Call("capi_cenv_clone", ns)
nms <- sort(ls(cl, all.names = TRUE))
print(nms)
# ordinary bindings to R and to C
print(vapply(nms, bindingIsActive, NA, cl))
print(.Call("capi_cenv_active", cl, nms))
print(tryCatch(activeBindingFunction("x", cl), error = conditionMessage))
print(tryCatch(makeActiveBinding("x", function() 1, cl), error = conditionMessage))
# the clone is a namespace, as GNU R's R_IsNamespaceEnv decides
print(isNamespace(cl))
print(identical(get(".__NAMESPACE__.", cl), info))
print(getNamespaceName(cl))
print(environmentName(cl))
print(identical(topenv(new.env(parent = cl)), cl))
print(cl$x)
print(cl$f())
# assigning from R replaces the value
cl$x <- 10
print(cl$x)
print(bindingIsActive("x", cl))
