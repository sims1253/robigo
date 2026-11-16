# The capi corpus's eval case (tests/corpora/capi/cases/eval.R) for
# robigo-rt, its C in support/eval.c built by scripts/rt-capi-build.sh
# into RT_CAPI_LIBS.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/eval.so"))

print(.Call("capi_eval"))
print(.Call("capi_tryeval"))
print(.Call("capi_parse"))
print(.Call("capi_option"))
