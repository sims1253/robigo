# #443: a value C keeps in a static from a lookup (rlang's
# `rlang_zap = rlang_ns_get("zap!")`, returned by ffi_env_poke and
# ffi_env_bind) must stay the bound object through collections; under
# gctorture a freed copy is reused at once and its conversion crashed.
dyn.load("lookup_static.so")

e <- new.env()
assign("marker", structure(list(), class = "marker"), envir = e)
assign("pair", c(a = 1.5, b = 2.5), envir = e)
invisible(.Call("capi_lookup_init", e))
gctorture2(1)
for (i in 1:3) {
    invisible(.Call("capi_lookup_churn", 50L))
    r <- .Call("capi_lookup_cached")
}
gctorture2(0)
print(r)
print(.Call("capi_lookup_is", e$marker, e$pair))
