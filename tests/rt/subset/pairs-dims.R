# `[` of pairlists and calls: coerced to lists, calls coerced back
# (robigo-rt does not print matrices yet: an array is shown as its
# attributes and its elements)
show <- function(expr) tryCatch({
    v <- expr
    if (is.array(v)) { print(attributes(v)); attributes(v) <- NULL; print(v) } else print(v)
}, error = function(e) print(e), warning = function(w) print(w))
p <- pairlist(a = 1, b = "x", 3)
show(p[2:3])
show(p[-1])
cl <- quote(f(x, y = 2, z))
show(cl[c(1, 3)])
show(cl[-1])
show(cl[0])
show(cl["y"])
pm <- pairlist(1, 2, 3, 4)
dim(pm) <- c(2L, 2L)
show(pm[1, ])
show(pm[, 2, drop = FALSE])
