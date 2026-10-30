# solve() with complex matrices: La_solve_cmplx (zgesv/zlange/zgecon),
# including the dimnames rules and the singular error paths.
options(digits = 15)

a <- matrix(c(1 + 1i, 2, 3, 1 - 1i), 2)

# Vector and matrix right-hand sides.
print(solve(a, c(1 + 1i, 2)))
print(solve(a, matrix(c(1 + 1i, 2, 3, 4i), 2)))

# Real b with complex a: coerced on the way in.
print(solve(a, c(1, 2)))

# Complex b with real a: La_solve_cmplx receives a coerced complex a.
print(solve(matrix(c(4, 1, 2, 3), 2), c(1 + 2i, 3)))

# Dimnames: the complex rule keeps them only when both sides have colnames.
an <- a
dimnames(an) <- list(NULL, c("x", "y"))
print(solve(an, c(1, 2)))
bn <- matrix(c(1 + 1i, 2, 3, 4i), 2)
dimnames(bn) <- list(NULL, c("p", "q"))
print(solve(an, bn))
print(solve(a, bn))

# Vector b keeps a's colnames as names.
print(names(solve(an, c(1 + 1i, 2))))

# Exactly singular.
sing <- matrix(c(1 + 1i, 2 + 2i, 3, 6), 2)
print(try(solve(sing, c(1, 1)), silent = TRUE))

# Computationally singular under a raised tolerance. Lapack.c:
# La_solve_cmplx takes the norm from COMPLEX(A): the original matrix for
# complex input, but the in-place factored buffer when A was coerced
# (real a, complex b) - the C's aliasing is observable in the message.
print(try(solve(a, c(1 + 1i, 2), tol = 10), silent = TRUE))
print(try(solve(matrix(c(4, 1, 2, 3), 2), c(1 + 2i, 3), tol = 10), silent = TRUE))

# solve(a) without b: the R wrapper inverts via a matrix identity.
print(solve(a))
