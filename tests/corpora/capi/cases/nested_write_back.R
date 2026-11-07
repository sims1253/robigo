dyn.load("nested_write_back.so")

# a nested call's write into an object the outer call has is the caller's
# as the nested call returns
l <- list(c(1L, 2L), 3L)
g <- function() {
    invisible(.Call("capi_nest_poke", l))
    print(l[[1]])
}
invisible(.Call("capi_nest_outer", l, g))
print(l[[1]])

# Nested calls cost what they are handed, not what the outer call holds:
# 1,000 calls inside one that holds a 50,000-element list.
big <- lapply(1:50000, function(i) c(i, i))
f <- function() for (k in 1:1000) .Call("capi_nest_inner", 1L)
print(system.time(.Call("capi_nest_outer", big, f))[["elapsed"]] < 3)
