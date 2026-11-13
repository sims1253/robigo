dyn.load("pin_extbuf.so")

# An external pointer held only by a C local, its finalizer freeing a
# buffer another C local still names (docs/g3.md §15.1): GNU R finalizes it
# at a full collection during the call, freeing the buffer.
print(.Call("capi_eb_probe"))
