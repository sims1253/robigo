# The capi corpus's pin_extbuf case (tests/corpora/capi/cases/pin_extbuf.R) for
# robigo-rt, its C in support/pin_extbuf.c built by scripts/rt-capi-build.sh
# into RT_CAPI_LIBS.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/pin_extbuf.so"))

# An external pointer held only by a C local, its finalizer freeing a
# buffer another C local still names (docs/g3.md §15.1): GNU R finalizes it
# at a full collection during the call, freeing the buffer.
print(.Call("capi_eb_probe"))
