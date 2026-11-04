dyn.load("newenv_bind.so")

info <- new.env()
info$spec <- c(name = "fakepkg", version = "1.0")
other <- new.env()
r <- .Call("capi_newenv_bind", info, "fakepkg", other)
e <- r[[1]]
print(r[[2]])  # R_IsNamespaceEnv
print(r[[3]])  # R_BindingIsActive
print(isNamespace(e))
print(environmentName(e))
# environments and symbols C binds are regular bindings (a vector C
# binds, like .packageName here, still reads as active in robigo: #341)
print(sapply(c(".__NAMESPACE__.", "an_env", "a_sym"), bindingIsActive, env = e))
print(identical(get(".__NAMESPACE__.", envir = e), info))
print(identical(e$an_env, other))
print(e$a_sym)
# topenv() stops at the copy, which counts as a namespace
child <- new.env(parent = e)
print(identical(topenv(child), e))
f <- function() topenv()
environment(f) <- child
print(identical(f(), e))
