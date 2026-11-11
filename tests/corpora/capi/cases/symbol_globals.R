dyn.load("symbol_globals.so")

# Every symbol global Rinternals.h declares is exported and installed.
print(.Call("capi_symbol_globals"))

# R_NamespaceEnvSymbol: a namespace's .__NAMESPACE__. information (base's
# namespace has none).
print(.Call("capi_namespace_name", asNamespace("stats")))
print(tryCatch(.Call("capi_namespace_name", asNamespace("base")),
               error = conditionMessage))

# R_AtsignSymbol: a call to `@` built in C.
x <- structure(1:3, slot1 = "a")
print(tryCatch(.Call("capi_atsign", x, quote(slot1), globalenv()),
               error = conditionMessage))
# an S3 method for `@` (R >= 4.3) receives the slot name as a string
`@.rec` <- function(object, name) paste0("rec@", name, "=", object[[name]])
r <- structure(list(a = 1), class = "rec")
print(.Call("capi_atsign", r, quote(a), globalenv()))
