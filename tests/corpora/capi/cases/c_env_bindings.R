dyn.load("c_env_bindings.so")

# What C binds with defineVar in an environment it made with R_NewEnv is a
# plain value binding, to R and to C, whatever its type.
vals <- list(num = c(1.5, 2.5), chr = character(), lst = list(1, "a"),
             fun = function(x) x + 1, env = globalenv(), sym = quote(s))
e <- .Call("capi_c_env_fill", emptyenv(), vals)
print(sort(names(e)))
for (n in sort(names(e))) cat(n, bindingIsActive(n, e), "\n")
print(.Call("capi_c_env_active", e, sort(names(e))))
print(e$num)
print(identical(e$env, globalenv()))
print(e$fun(1))
print(tryCatch(activeBindingFunction("num", e), error = conditionMessage))
print(tryCatch(makeActiveBinding("num", function() 1, e), error = conditionMessage))

# C still writes into the double it bound, and R sees it.
.Call("capi_c_env_poke", e, quote(num), 9)
print(e$num)
e$num <- 3
print(e$num)
print(bindingIsActive("num", e))

# A clone of a namespace is a namespace (R_IsNamespaceEnv looks at its
# .__NAMESPACE__.), and topenv() stops there.
ns <- asNamespace("stats")
cl <- .Call("capi_c_env_clone", ns)
print(isNamespace(cl))
print(environmentName(cl))
print(identical(topenv(new.env(parent = cl)), cl))
print(getNamespaceName(cl))
print(bindingIsActive(".__NAMESPACE__.", cl))
