dyn.load("newenv_bind.so")

info <- new.env()
info$spec <- c(name = "fakepkg", version = "1.0")
other <- new.env()
fun <- function(x) x + 1
r <- .Call("capi_newenv_bind", info, "fakepkg", other, fun)
e <- r[[1]]
print(r[[2]])  # R_IsNamespaceEnv
print(r[[3]])  # R_BindingIsActive
print(isNamespace(e))
print(environmentName(e))
# environments, symbols and functions C binds are regular bindings (a vector C
# binds, like .packageName here, still reads as active in robigo: #341)
print(sapply(c(".__NAMESPACE__.", "an_env", "a_sym", "a_fun", "a_prim"),
             bindingIsActive, env = e))
print(identical(get(".__NAMESPACE__.", envir = e), info))
print(identical(e$an_env, other))
print(e$a_sym)
print(identical(e$a_fun, fun))
print(e$a_fun(41))
print(e$a_prim(1, 2))
# topenv() stops at the copy, which counts as a namespace
child <- new.env(parent = e)
print(identical(topenv(child), e))
f <- function() topenv()
environment(f) <- child
print(identical(f(), e))
