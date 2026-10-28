# u %*% M for a row vector u (dgemv "T"): exact sums, signed zeros, NA/Inf.

u <- sin(1:7) / 3
M <- matrix(cos((1:35) * 1.7) * 7.3, 7)
r <- u %*% M
print(r, digits = 17)
print(sprintf("%a", r))
print(identical(r, matrix(u, 1) %*% M))
print(sprintf("%a", crossprod(u, M)))
z <- c(-0, -0) %*% matrix(c(1, 2, 3, 4), 2)
print(sprintf("%a", z))
print(1 / z)
print(c(1, NA) %*% matrix(1:4, 2))
print(c(1, Inf) %*% matrix(c(0, 1, 2, 3), 2))
print(matrix(1:3, 1) %*% matrix(1:3, 3))
print(1:3 %*% matrix(as.numeric(1:6), 3))
big <- c(1e308, 1e308, -1e308)
print(big %*% matrix(1, 3, 2))
print(c(1e16, 1, -1e16) %*% matrix(1, 3, 2))
