# x[m]: an array indexed by a matrix with a column per extent
# (robigo-rt does not print matrices yet: an array is shown as its
# attributes and its elements)
show <- function(expr) tryCatch({
    v <- expr
    if (is.array(v)) { print(attributes(v)); attributes(v) <- NULL; print(v) } else print(v)
}, error = function(e) print(e), warning = function(w) print(w))
# (robigo-rt has no cbind/rbind yet: these build the same matrices)
cb <- function(...) { a <- list(...); matrix(unlist(a), ncol = length(a)) }
rb <- function(...) { a <- list(...); matrix(unlist(a), nrow = length(a), byrow = TRUE) }
m <- matrix(1:12, 3, 4)
i <- cb(c(1, 2, 3), c(4, 1, 2))
show(m[i])
show(m[cb(2L, 3L)])
show(m[cb(c(1, NA), c(2, 2))])
show(m[cb(c(1, 2), c(NA, 2))])
show(m[cb(c(1, 0), c(1, 2))])
show(m[cb(c(0, NA), c(1, 2))])
show(m[cb(c(2.9, 1.1), c(1, 2))])
show(m[matrix(integer(0), 0, 2)])
# a matrix with another number of columns is a vector subscript
show(m[cb(1:2)])
show(m[matrix(1:6, 2, 3)])
# a logical matrix is a logical subscript
show(m[matrix(c(TRUE, FALSE), 3, 4)])
# names are not kept
mn <- m
dimnames(mn) <- list(letters[1:3], LETTERS[1:4])
show(mn[cb(1:2, 2:3)])
# character matrices match the dimnames
show(mn[cb(c("a", "c"), c("D", "A"))])
show(mn[cb(c("a", NA), c("B", "B"))])
a <- array(1:24, c(2, 3, 4), dimnames = list(c("x", "y"), NULL, c("p", "q", "r", "s")))
show(a[cb(2, 3, 4)])
show(a[rb(c(1, 1, 1), c(2, 2, 2), c(1, 3, 4))])
show(a[cb(c(1, 2), c(0, 1), c(1, 1))])
# a factor matrix is a vector subscript
f <- factor(c("b", "a"))
dim(f) <- c(1L, 2L)
show(m[f])
# 1-d arrays
o <- array(10:13)
show(o[cb(c(2, 4))])
show(o[matrix(c(2, 4), 1)])
# errors
show(m[cb(-1, 2)])
show(m[cb(4, 1)])
show(m[cb(1, 5)])
show(a[cb(1, 4, 1)])
show(mn[cb("a", "E")])
show(mn[cb("", "A")])
show(m[cb("a", "b")])
show(a[cb("x", "q", "p")])
show(m[cb(1, 2e10)])
