# x[m] <- y: an array assigned through a matrix index
show <- function(expr) tryCatch(print(expr), error = function(e) print(e), warning = function(w) print(w))
m <- matrix(0L, 3, 4)
m[cbind(1:3, c(4, 1, 2))] <- 1:3; show(m)
m[cbind(c(1, NA), c(1, 1))] <- 9L; show(m)
m[cbind(c(0, 2), c(1, 1))] <- 7L; show(m)
m[cbind(2.9, 3.1)] <- -1L; show(m)
dimnames(m) <- list(letters[1:3], LETTERS[1:4])
m[cbind(c("a", "c"), c("D", "A"))] <- c(10L, 20L); show(m)
m[cbind(c("b", NA), c("B", "B"))] <- 30L; show(m)
a <- array(0, c(2, 3, 2))
a[rbind(c(1, 1, 1), c(2, 3, 2))] <- c(5, 6); show(a)
# a matrix with another number of columns is a vector subscript
v <- 1:10
v[cbind(1:2)] <- 0L; show(v)
mm <- matrix(0, 2, 2)
mm[matrix(c(TRUE, FALSE), 2, 2)] <- 1; show(mm)
mm[matrix(1:3, 1, 3)] <- 2; show(mm)
# grows a vector, not an array
show({ mm[cbind(3, 1)] <- 1; mm })
# errors
mm <- matrix(0, 2, 2)
show({ mm[cbind(-1, 1)] <- 1; mm })
show({ mm[cbind("a", "b")] <- 1; mm })
dimnames(mm) <- list(c("a", "b"), c("c", "d"))
show({ mm[cbind("a", "e")] <- 1; mm })
show({ mm[cbind(c(1, NA), 1)] <- 1:2; mm })
show({ mm[cbind(1, 3)] <- 1; mm })
