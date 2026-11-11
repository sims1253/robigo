dyn.load("api_symbols.so")

# R_AtsignSymbol and R_NamespaceEnvSymbol (Rinternals.h)
print(.Call("capi_api_symbols"))
