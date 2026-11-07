# Extents with names (dim = c(a = 2L, ...), as a deserialized array may
# have) name the extents of a slice in GNU R (subset.c:ArraySubset copies
# the dim vector's names). A gap: robigo's `[` drops them.
nd <- structure(1:8 / 2, dim = c(a = 2L, b = 2L, c = 2L))
print(attributes(nd[, , 1]))
