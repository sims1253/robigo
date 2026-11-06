# kappa(., exact = TRUE) on triangular matrices goes through .Fortran(.F_dtrco)
# (appl/dtrco.f's LINPACK estimate), upper and lower.
x1 <- matrix(c(1, 1, 0, 0, 1, 1, 1, 1, 1), 3, 3)
print(kappa(z <- qr(x1), exact = FALSE))
print(kappa(z, exact = TRUE))
x.hil <- diag(1:12) + 1
print(kappa(qr(x.hil), exact = TRUE))
print(kappa(x1, exact = TRUE))
u <- matrix(c(1, 0, 0, 2, 3, 0, 0, 4, 5), 3, 3)
print(kappa(u, exact = TRUE, uplo = "U"))
l <- t(u)
print(kappa(l, exact = TRUE, uplo = "L"))
# a triangular matrix with a zero diagonal entry: rcond 0, kappa Inf
print(kappa(matrix(c(0, 1, 0, 2), 2, 2), exact = TRUE, uplo = "U"))
