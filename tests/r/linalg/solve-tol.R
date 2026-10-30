# The rcond path of La_solve: Lapack.c takes the 1-norm from REAL(A) -
# the original matrix for double input (avals is a copy), but the
# in-place factored buffer when A was coerced from integer/logical.
# The difference is observable in the "computationally singular" message.
options(digits = 15)

ai <- matrix(1:4, 2)
print(try(solve(ai, 8:9, tol = 1), silent = TRUE))
print(try(solve(ai + 0, c(8, 9), tol = 1), silent = TRUE))
print(try(solve(matrix(c(TRUE, FALSE, FALSE, TRUE), 2), c(1, 1), tol = 1), silent = TRUE))

# Below tolerance the check is silent and the solution is the same.
print(solve(ai, 8:9))
print(solve(ai + 0, c(8, 9)))
