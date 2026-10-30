# Random-matrix battery: every LAPACK internal eigen()/svd()/solve() uses,
# printed at full precision. Deterministic LCG (base only; stats' rnorm is
# unavailable under R_DEFAULT_PACKAGES=NULL). Recorded from GNU R 4.5.0.
options(digits = 17)
seed <- as.double(20261030)
lcg <- function() { seed <<- (48271 * seed) %% 2147483647; seed / 2147483647 }
rval <- function(k) sapply(seq_len(k), function(i) lcg() * 2 - 1)
rmat <- function(n, p) matrix(rval(n * p), n, p)
for (n in c(1, 2, 3, 5, 8, 12)) {
  a <- rmat(n, n)
  s <- a + t(a)                    # symmetric
  cat("## n =", n, "\n")
  print(eigen(s)$values)
  print(eigen(s)$vectors)
  print(eigen(s, only.values = TRUE)$values)
  print(eigen(a)$values)           # general real: may be complex
  if (is.complex(eigen(a)$values)) print(eigen(a)$vectors)
  z <- a + (0+1i) * t(a) / 2       # general complex
  zs <- (z + Conj(t(z))) / 2       # Hermitian
  print(eigen(zs)$values)
  print(eigen(zs, only.values = TRUE)$values)
  if (n >= 2) print(svd(a)$d)
  if (n >= 2) print(svd(a)$u)
  if (n >= 2) print(svd(a, nu = 0)$d)
  if (n >= 2) print(svd(z)$d)
  if (n >= 2) print(svd(z, nu = 0)$d)
  print(solve(a + diag(n) * n))    # well-conditioned solve
  print(solve(a + diag(n) * n, cbind(rev(seq_len(n)), seq_len(n))))
  print(solve(z + diag(n) * n))
  print(det(a + diag(n) * n))
}
## singular / rank-deficient cases
sing <- matrix(c(1, 2, 2, 4), 2, 2)
print(svd(sing)$d)
print(tryCatch(solve(sing), error = conditionMessage))
print(eigen(sing, only.values = TRUE)$values)
sing6 <- rmat(6, 6); sing6[, 6] <- sing6[, 1] * 2 - sing6[, 3]
print(svd(sing6)$d)
print(eigen(sing6, only.values = TRUE)$values)
## chol with pivot
p5 <- crossprod(rmat(5, 5))
print(chol(p5, pivot = TRUE))
print(chol(p5))
