# u %*% v for two vectors (a 1 x 1 result): GNU R's dgemv for finite
# operands, simple_matprod otherwise; NA and NaN payloads, infinities,
# signed zeros.
show <- function(m) print(c(m, is.na(m), is.nan(m)))
x <- c(1, 2, 3); y <- c(4, 5, 6)
show(x %*% y); print(dim(x %*% y))
show(c(0, -0) %*% c(-1, 1))
show(c(-0, -0) %*% c(1, 1))
show(c(NA, 1) %*% c(NaN, 1))
show(c(NaN, 1) %*% c(NA, 1))
show(c(1, NA) %*% c(1, NaN))
show(c(1, 2) %*% c(NA, NaN))
show(c(Inf, 1) %*% c(0, 1))
show(c(Inf, -Inf) %*% c(1, 1))
show(c(1e308, 1e308) %*% c(10, -10))
show(1:3 %*% c(0.5, NA, 2))
# A sum that is already NaN meets another NaN: the sum's payload survives.
show(c(1, 2, 2, 3) %*% c(1, 3, NaN, NA))
show(c(1, 2, 2, 3) %*% c(1, 3, NA, NaN))
show(c(1, 2, NA, NaN) %*% c(1, 3, 3, 4))
show(c(1, 2, NaN, NA) %*% c(1, 3, 3, 4))
show(c(NA, NaN) %*% c(NaN, NA))
show(matrix(c(1, 2), 1) %*% matrix(c(3, NA), 2))
show(numeric(0) %*% numeric(0))
# `[[<-` of a 1 x 1 matrix (attributes dropped), names, and an object.
r <- double(3)
r[[2]] <- x %*% y
print(r)
r[[3]] <- c(a = 7)
print(r); print(names(r))
r[[1]] <- structure(9, class = "foo")
print(r)
f <- function(n) { out <- double(n); for (i in 1:n) out[[i]] <- (1:3 + i) %*% (3:1); out }
print(f(5)); print(f(5))
