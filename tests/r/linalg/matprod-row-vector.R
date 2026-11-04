# A vector times a matrix (array.c:matprod: dgemv("T") when both are
# finite, simple_matprod otherwise): the same sums in the same order, so
# the same bits whatever the matrix holds, NaN payloads and signed zeros
# included. Columns run eight at a time, the rest one by one.

n <- 37
y <- matrix(sin(seq_len(n * 21) * 1.7) * 10^(seq_len(n * 21) %% 7 - 3), n, 21)
x <- cos(seq_len(n) * 2.3) / 3
z <- x %*% y
print(dim(z)); print(sprintf("%.17g", z))
y2 <- y
y2[3, 2] <- NA; y2[5, 9] <- NaN; y2[1, 17] <- Inf; y2[2, 17] <- -Inf
y2[4, 20] <- Inf; y2[7, 21] <- NA; y2[8, 21] <- NaN
print(x %*% y2)
x0 <- x; x0[c(3, 5)] <- 0
print(x0 %*% y2)
# non-finite in the vector
x2 <- x; x2[4] <- NA
print(x2 %*% y2)
x3 <- x; x3[4] <- NaN
print(x3 %*% y2)
x4 <- x; x4[6] <- Inf
print(x4 %*% y2)
# signed zeros
print(sprintf("%g", 1 / (c(-0, -0) %*% matrix(c(0, 0, -0, -0, 1, -1), 2))))
# overflow to Inf from finite values
print(c(1e308, 1e308) %*% matrix(c(1, 1, 2, -1, 1, -1, 0, 0, 3), 2))
# integers and logicals
print(1:4 %*% matrix(1:20, 4)); print(c(TRUE, NA) %*% matrix(1:6, 2))
print(1:9 %*% matrix(as.numeric(1:81), 9))
