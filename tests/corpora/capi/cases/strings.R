dyn.load("strings.so")

print(.Call("capi_mk", c("truncated", "via R_CHAR")))
print(.Call("capi_ce"))                 # encodings, plus the list element is a UTF-8 truncation
print(.Call("capi_translate"))          # both end up as UTF-8 in a UTF-8 locale
print(.Call("capi_reencode"))
print(.Call("capi_asChar", 42L))
print(.Call("capi_asChar", 1.5))
print(.Call("capi_asChar", TRUE))
print(.Call("capi_asChar", NULL))
print(.Call("capi_ascii", c("plain", "caf\xc3\xa9", "")))
print(.Call("capi_nchar", "caf\xc3\xa9"))
