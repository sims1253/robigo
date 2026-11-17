# x[m]: an array indexed by a matrix with a column per extent
show <- function(expr) tryCatch(print(expr), error = function(e) print(e), warning = function(w) print(w))
m <- matrix(1:12, 3, 4)
i <- cbind(c(1, 2, 3), c(4, 1, 2))
show(m[i])
show(m[cbind(2L, 3L)])
show(m[cbind(c(1, NA), c(2, 2))])
show(m[cbind(c(1, 2), c(NA, 2))])
show(m[cbind(c(1, 0), c(1, 2))])
show(m[cbind(c(0, NA), c(1, 2))])
show(m[cbind(c(2.9, 1.1), c(1, 2))])
show(m[matrix(integer(0), 0, 2)])
# a matrix with another number of columns is a vector subscript
show(m[cbind(1:2)])
show(m[matrix(1:6, 2, 3)])
# a logical matrix is a logical subscript
show(m[matrix(c(TRUE, FALSE), 3, 4)])
# names are not kept
mn <- m
dimnames(mn) <- list(letters[1:3], LETTERS[1:4])
show(mn[cbind(1:2, 2:3)])
# character matrices match the dimnames
show(mn[cbind(c("a", "c"), c("D", "A"))])
show(mn[cbind(c("a", NA), c("B", "B"))])
a <- array(1:24, c(2, 3, 4), dimnames = list(c("x", "y"), NULL, c("p", "q", "r", "s")))
show(a[cbind(2, 3, 4)])
show(a[rbind(c(1, 1, 1), c(2, 2, 2), c(1, 3, 4))])
show(a[cbind(c(1, 2), c(0, 1), c(1, 1))])
# a factor matrix is a vector subscript
f <- factor(c("b", "a"))
dim(f) <- c(1L, 2L)
show(m[f])
# 1-d arrays
o <- array(10:13)
show(o[cbind(c(2, 4))])
show(o[matrix(c(2, 4), 1)])
# errors
show(m[cbind(-1, 2)])
show(m[cbind(4, 1)])
show(m[cbind(1, 5)])
show(a[cbind(1, 4, 1)])
show(mn[cbind("a", "E")])
show(mn[cbind("", "A")])
show(m[cbind("a", "b")])
show(a[cbind("x", "q", "p")])
show(m[cbind(1, 2e10)])
