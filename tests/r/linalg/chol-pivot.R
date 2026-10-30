# chol(pivot = TRUE): dpstrf through the system-LAPACK bridge, including
# the rank-deficient warning path and pivoted colnames.
options(digits = 15)

m <- matrix(c(2, .2, .5, .2, 3, 1, .5, 1, 4), 3)
print(chol(m, pivot = TRUE))
print(attributes(chol(m, pivot = TRUE)))

# Colnames are permuted by the pivot.
mn <- m
dimnames(mn) <- list(letters[1:3], LETTERS[1:3])
print(chol(mn, pivot = TRUE))

# Rank-deficient: warning plus the residual the routine leaves behind.
r <- chol(matrix(c(4, 2, 2, 1, 0, 0, 0, 0, 1), 3), pivot = TRUE)
print(unclass(r))
print(attributes(r)$pivot)
print(attributes(r)$rank)

# Not positive definite at all.
print(try(chol(matrix(c(-1, 0, 0, -1), 2), pivot = TRUE), silent = TRUE))

# Explicit tolerances.
print(attr(chol(m, pivot = TRUE, tol = 1e-8), "rank"))
print(attr(chol(m, pivot = TRUE, tol = 1e300), "rank"))

# Non-pivoting chol still routes through the reference port.
print(chol(m))
print(try(chol(matrix(c(1, 2, 2, 1), 2)), silent = TRUE))
print(try(chol(m, pivot = 2), silent = TRUE))
