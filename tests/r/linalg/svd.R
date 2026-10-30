# svd(): square, thin, no vectors, rank-deficient (singular), complex —
# the La_svd/La_svd_cmplx internals over dgesdd/zgesdd.
options(digits = 17)

## square
a <- matrix(c(1, 2, 3, 4, 5, 6, 7, 8, 10), 3, 3)
s <- svd(a)
print(s$d)
print(s$u)
print(s$v)

## rectangular thin decomposition
x <- cbind(1, 1:7)
print(svd(x)$d)
print(svd(x)$u)
print(svd(x)$v)

## no left/right vectors
print(svd(x, nu = 0)$d)
print(svd(x, nu = 0, nv = 0)$u)

## rank-deficient: last singular value collapses to ~0
sing <- cbind(x, 3 * x[, 1] + 2 * x[, 2])
ss <- svd(sing)
print(ss$d)

## smallest singular value of a near-singular matrix
ns <- matrix(c(1, 1, 1, 1 + 1e-12), 2, 2)
print(svd(ns)$d)

## complex
zc <- matrix(c(1+0i, 1i, -1i, 2+0i), 2, 2)
print(svd(zc)$d)
print(svd(zc)$u)
