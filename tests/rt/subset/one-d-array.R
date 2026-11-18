# 1-d arrays through VectorSubset: the dim kept unless dropped
# (robigo-rt does not print matrices yet: an array is shown as its
# attributes and its elements)
show <- function(expr) tryCatch({
    v <- expr
    if (is.array(v)) { print(attributes(v)); attributes(v) <- NULL; print(v) } else print(v)
}, error = function(e) print(e), warning = function(w) print(w))
o <- array(1:4)
show(attributes(o[2:3]))
show(attributes(o[2]))
show(attributes(o[2, drop = FALSE]))
show(attributes(o[]))
on <- array(1:4, dimnames = list(c("a", "b", "c", "d")))
show(on[2:3])
show(attributes(on[2:3]))
show(on["b"])
show(attributes(on["b", drop = FALSE]))
show(on[c("d", "a")])
names(dimnames(on)) <- "k"
show(attributes(on[c(1, 3)]))
show(attributes(on[-1]))
d <- array(1:3, c(n = 3L))
show(attributes(d[1:2]))
show(attributes(d[0]))
show(attributes(on[0]))
show(on[5])
show(attributes(on[c(1, NA)]))
