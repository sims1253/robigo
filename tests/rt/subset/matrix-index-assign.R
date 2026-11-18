# x[m] <- y: an array assigned through a matrix index
# (robigo-rt does not print matrices yet: an array is shown as its
# attributes and its elements)
show <- function(expr) tryCatch({
    v <- expr
    if (is.array(v)) { print(attributes(v)); attributes(v) <- NULL; print(v) } else print(v)
}, error = function(e) print(e), warning = function(w) print(w))
# (robigo-rt has no cbind/rbind yet: these build the same matrices)
cb <- function(...) { a <- list(...); matrix(unlist(a), ncol = length(a)) }
rb <- function(...) { a <- list(...); matrix(unlist(a), nrow = length(a), byrow = TRUE) }
m <- matrix(0L, 3, 4)
m[cb(1:3, c(4, 1, 2))] <- 1:3; show(m)
m[cb(c(1, NA), c(1, 1))] <- 9L; show(m)
m[cb(c(0, 2), c(1, 1))] <- 7L; show(m)
m[cb(2.9, 3.1)] <- -1L; show(m)
dimnames(m) <- list(letters[1:3], LETTERS[1:4])
m[cb(c("a", "c"), c("D", "A"))] <- c(10L, 20L); show(m)
m[cb(c("b", NA), c("B", "B"))] <- 30L; show(m)
a <- array(0, c(2, 3, 2))
a[rb(c(1, 1, 1), c(2, 3, 2))] <- c(5, 6); show(a)
# a matrix with another number of columns is a vector subscript
v <- 1:10
v[cb(1:2)] <- 0L; show(v)
mm <- matrix(0, 2, 2)
mm[matrix(c(TRUE, FALSE), 2, 2)] <- 1; show(mm)
mm[matrix(1:3, 1, 3)] <- 2; show(mm)
# grows a vector, not an array
show({ mm[cb(3, 1)] <- 1; mm })
# errors
mm <- matrix(0, 2, 2)
show({ mm[cb(-1, 1)] <- 1; mm })
show({ mm[cb("a", "b")] <- 1; mm })
dimnames(mm) <- list(c("a", "b"), c("c", "d"))
show({ mm[cb("a", "e")] <- 1; mm })
show({ mm[cb(c(1, NA), 1)] <- 1:2; mm })
show({ mm[cb(1, 3)] <- 1; mm })
