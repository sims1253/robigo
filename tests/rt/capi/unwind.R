# The capi corpus's unwind case (tests/corpora/capi/cases/unwind.R) for
# robigo-rt, its C in support/unwind.c built by scripts/rt-capi-build.sh
# into RT_CAPI_LIBS.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/unwind.so"))

print(tryCatch(.Call("capi_unwind"),
               error = function(e) paste("caught:", conditionMessage(e))))
print(.Call("capi_unwind_ok"))
print(.Call("capi_execcleanup"))
print(.Call("capi_toplevelexec"))
