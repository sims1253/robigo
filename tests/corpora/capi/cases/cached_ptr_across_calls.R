dyn.load("cached_ptr_across_calls.so")

# C caches INTEGER(x) in one call and writes through the pointer in a
# later call that is not handed x: R sees the write.
x <- .Call("capi_fresh_pair")
invisible(.Call("capi_cache_ptr", x))
invisible(.Call("capi_write_cached", 99L))
x
invisible(.Call("capi_write_cached", 7L))
x
