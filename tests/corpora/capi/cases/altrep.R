dyn.load("altrep.so")

q <- .Call("capi_altrep_queries", c("wrap", "ped", "strings"))
print(q[1:5])
print(q[[6]])
print(q[[7]])
print(q[[8]])

x <- .Call("capi_altrep_seq")
print(x)
print(length(x))
print(x[2:4])
print(sum(x))
print(identical(x, c(10L, 12L, 14L, 16L, 18L)))
print(as.integer(x))  # materialization through DATAPTR
print(x)              # still the same values after materializing

s <- .Call("capi_altrep_strs", c("a", "b", "c"))
print(s)
s[1] <- "A"  # Set_elt method mutates the wrapped data
print(s)
