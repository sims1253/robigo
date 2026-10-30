# svd(): La_svd (dgesdd) and La_svd_cmplx (zgesdd), every jobu the R
# wrapper can produce, plus the singular case.
options(digits = 15)

sym <- matrix(c(5, 1, 2, 1, 4, 1, 2, 1, 3), 3)
wide <- matrix(c(1.5, -2.5, 3.5, -4.5, 5.5, -6.5, 7.5, -8.5, 9.5, -10.5, 11.5, -12.5), 3)
tall <- matrix(c(1.5, -2.5, 3.5, -4.5, 5.5, -6.5, 7.5, -8.5, 9.5, -10.5, 11.5, -12.5), 4)

# Square, wide and tall: jobu "S" through the default nu/nv.
print(svd(sym))
print(svd(wide))
print(svd(tall))

# jobu "N" for u, for v, and for both.
print(svd(wide, nu = 0))
print(svd(tall, nv = 0))
print(svd(wide, nu = 0, nv = 0)$d)

# jobu "A": full U and V.
print(svd(wide, nu = 4, nv = 4))

# Complex input.
z <- matrix(c(1 + 2i, 3, 1i, 2 - 1i, 0, 1, 5, 2i), 4)
print(svd(z))

# A singular matrix: the last singular value is numerical noise.
print(svd(matrix(c(1, 2, 2, 4), 2))$d)

# 1x1 and a vector-shaped input via as.matrix.
print(svd(matrix(7, 1)))
print(svd(1:6))

# Errors.
print(try(svd(matrix(numeric(), 0, 2)), silent = TRUE))
print(try(svd(matrix(c(1, NA), 1)), silent = TRUE))
print(try(svd(matrix(c(1, Inf), 1)), silent = TRUE))
