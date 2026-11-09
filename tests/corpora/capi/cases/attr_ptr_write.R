dyn.load("attr_ptr_write.so")

# A compact sequence C expanded and bound (`bound` shares its buffer),
# then a later call swaps its names through DATAPTR_RO.
invisible(.Call("capi_bind_named", seq_len(3), environment()))
bound
invisible(.Call("capi_swap_names_ro", bound))
names(bound)
bound

# The same with a plain vector C bound (a zero-copy object).
invisible(.Call("capi_bind_named", c(5L, 6L, 7L), environment()))
invisible(.Call("capi_swap_names_ro", bound))
names(bound)
bound

# And with a vector R named and bound itself.
v <- c(x = 1L, y = 2L, z = 3L)
invisible(.Call("capi_swap_names_ro", v))
names(v)
