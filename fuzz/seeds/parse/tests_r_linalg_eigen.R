# eigen(): La_rs (dsyevr), La_rg (dgeev) and their complex variants,
# through the system-LAPACK bridge (GNU R binds the same liblapack.so.3).
options(digits = 15)

# Symmetric, full decomposition.
sym <- matrix(c(5, 1, 2, 1, 4, 1, 2, 1, 3), 3)
print(eigen(sym))
print(eigen(sym, only.values = TRUE))

# A graded spectrum: the Hilbert matrix, well conditioned to near zero.
hilb <- outer(1:12, 1:12, function(i, j) 1 / (i + j - 1))
print(eigen(hilb, only.values = TRUE)$values)

# Non-symmetric with real eigenvalues.
ns <- matrix(c(1, 2, 3, 4, 5, 6, 7, 8, 10), 3) + diag(c(.1, -.2, .3))
print(eigen(ns))
print(eigen(ns, only.values = TRUE))

# Non-symmetric with a complex pair: unscramble() makes complex vectors.
rot <- matrix(c(0, -1, 1, 0), 2)
print(eigen(rot))
print(eigen(rot, only.values = TRUE))
print(eigen(rot)$vectors)

# Complex Hermitian (zheev) and complex general (zgeev).
h <- matrix(c(1 + 0i, 2 - 1i, 2 + 1i, 3 + 0i), 2)
print(eigen(h))
g <- matrix(c(1 + 2i, 3, 1i, 2 - 1i), 2)
print(eigen(g))
print(eigen(g)$vectors)

# eigen() reorders by modulus for the non-symmetric case.
print(eigen(matrix(c(1, 3, 2, 4), 2))$values)

# Errors.
print(try(eigen(matrix(1:6, 2)), silent = TRUE))
print(try(eigen(matrix(numeric(), 0, 0)), silent = TRUE))
print(try(eigen(matrix(c(1, NaN), 1)), silent = TRUE))
print(try(eigen(matrix(c(1, 2, 3, Inf), 2)), silent = TRUE))
