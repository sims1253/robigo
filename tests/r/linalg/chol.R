# chol()/chol2inv()/solve()/det()/rcond(): the La_chol, La_chol2inv,
# La_solve, det_ge_real and La_dgecon internals, with singular and
# non-positive-definite error texts.
options(digits = 17)

a <- matrix(c(5, 1, 2, 1, 4, 3, 2, 3, 6), 3, 3)
ch <- chol(a)
print(ch)
print(chol2inv(ch))
print(all.equal(a, crossprod(ch)))

## not positive definite
npd <- matrix(c(1, 2, 3, 4), 2, 2)
print(tryCatch(chol(npd), error = conditionMessage))

## exactly singular solve (real)
sing <- matrix(c(1, 2, 2, 4), 2, 2)
print(tryCatch(solve(sing), error = conditionMessage))
print(tryCatch(solve(sing, tol = 0), error = conditionMessage))

## complex solve (La_solve_cmplx over zgesv)
az <- matrix(c(1+1i, 2, 3, 4-1i), 2, 2)
bz <- c(1+0i, 2+1i)
print(solve(az, bz))
print(solve(az, c(1, 1)))

## determinant and condition numbers
print(det(a))
print(rcond(a))
print(rcond(a, "I"))
print(kappa(a, exact = TRUE))
