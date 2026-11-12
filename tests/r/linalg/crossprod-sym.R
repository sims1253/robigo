# crossprod(x) and tcrossprod(x) with y missing: GNU R's symcrossprod and
# symtcrossprod (dsyrk with the upper triangle reflected) for finite x,
# simple_crossprod/simple_tcrossprod (no reflection) otherwise; the
# two-argument forms (dgemm, dgemv) for comparison.
bits <- function(m) sprintf("%a", m)
show <- function(m) { print(m); print(c(is.na(m), is.nan(m))) }
# Values whose sums are order-sensitive in the last bits.
x <- matrix(sin(1:600 * 1.7) * 10^((1:600 * 7) %% 9 - 4), 40, 15)
s <- crossprod(x)
print(dim(s)); print(isSymmetric(s)); print(identical(s, crossprod(x, x)))
print(identical(s, t(x) %*% x))
print(bits(s[c(1, 2, 16, 17, 31, 225)]))
st <- tcrossprod(x)
print(dim(st)); print(isSymmetric(st)); print(identical(st, tcrossprod(x, x)))
print(identical(st, x %*% t(x)))
print(bits(st[c(1, 2, 41, 42, 801, 1600)]))
# Sizes past the blocking: 300 x 70 (crossprod) and 70 x 300 (tcrossprod).
y <- matrix(sin(1:21000) * 10^((1:21000) %% 7 - 3), 300, 70)
print(bits(sum(crossprod(y)))); print(bits(crossprod(y)[c(1, 70, 71, 4900)]))
print(identical(crossprod(y), crossprod(y, y)))
print(bits(sum(tcrossprod(t(y))))); print(identical(tcrossprod(t(y)), crossprod(y)))
print(bits(sum(y %*% t(y)))); print(identical(y %*% t(y), tcrossprod(y)))
z <- matrix(cos(1:4200), 70, 60)
w <- matrix(cos(1:15000) * 3^((1:15000) %% 5), 300, 50)
print(bits(crossprod(y, w)[c(1, 2, 3500)])); print(bits(sum(crossprod(y, w))))
print(bits(sum(crossprod(z, y[1:70, ])))); print(bits(sum(tcrossprod(z, z[1:30, ]))))
# Short inner dimensions (outer() is tcrossprod(x, y) of two vectors).
u <- sin(1:583 * 0.3) * 10^((1:583) %% 5 - 2); v <- cos(1:400 * 0.7) * 10^((1:400) %% 3)
o <- outer(u, v)
print(dim(o)); print(bits(sum(o))); print(bits(o[c(1, 583, 584, 233200)]))
print(identical(o, u %*% t(v))); print(identical(o, tcrossprod(u, v)))
for (k in c(2, 3, 9, 10)) {
  a <- matrix(sin(1:(300 * k) * 1.3) * 10^((1:(300 * k)) %% 7 - 3), 300, k)
  b <- matrix(cos(1:(k * 200)) * 3^((1:(k * 200)) %% 5), k, 200)
  print(c(k, bits(sum(a %*% b)), bits(sum(tcrossprod(a))), bits(sum(crossprod(t(a))))))
  print(bits(c((a %*% b)[c(1, 300, 59999)], tcrossprod(a)[c(2, 301, 89999)])))
  print(c(identical(tcrossprod(a), tcrossprod(a, a)), identical(crossprod(t(a)), tcrossprod(a)),
          identical(crossprod(t(a), b), a %*% b), isSymmetric(tcrossprod(a))))
}
# Non-finite input: NA, NaN, Inf. No reflection; payloads per element.
m <- matrix(c(1, NA, 3, NaN, 5, 6, Inf, 8, -Inf, 0, 2, -1), 4, 3)
show(crossprod(m)); show(tcrossprod(m))
show(crossprod(m, m)); show(tcrossprod(m, m))
m2 <- matrix(c(NA, NaN, 1, 2, NaN, NA, 3, 4), 2)
show(crossprod(m2)); show(tcrossprod(m2))
show(crossprod(matrix(c(Inf, 1, 1, 1), 2)))
show(crossprod(matrix(c(1e200, 1e200, 1e200, -1e200), 2)))
show(tcrossprod(matrix(c(1e200, 1e200, -1e200, 1e200), 2)))
# Signed zeros.
show(crossprod(matrix(c(-0, -0, 0, -0), 2))); show(tcrossprod(matrix(c(-0, 0, -0, -0), 2)))
# Dimnames and their names.
d <- matrix(1:6 / 7, 3, dimnames = list(rows = c("a", "b", "c"), cols = c("u", "v")))
print(crossprod(d)); print(tcrossprod(d))
d2 <- matrix(1:6 / 7, 3, dimnames = list(NULL, c("u", "v")))
print(crossprod(d2)); print(tcrossprod(d2))
d3 <- matrix(1:6 / 7, 3, dimnames = list(c("a", "b", "c"), NULL))
print(crossprod(d3)); print(tcrossprod(d3))
print(crossprod(d, d2)); print(tcrossprod(d3, d))
# Vectors.
v <- c(a = 1.5, b = -2, c = 3)
print(crossprod(v)); print(tcrossprod(v)); print(crossprod(2)); print(tcrossprod(2))
print(crossprod(c(1, NA, 3))); print(tcrossprod(c(1, NA, 3)))
print(crossprod(1:3, matrix(1:6, 3))); print(tcrossprod(1:3, matrix(1:6, 2)))
print(crossprod(matrix(1:6, 3), 1:3)); print(tcrossprod(matrix(1:3, 1), 1:3)); print(tcrossprod(matrix(1:3, 3), 1:3))
# Integer and logical input.
i <- matrix(1:12, 4)
print(crossprod(i)); print(tcrossprod(i)); print(storage.mode(crossprod(i)))
print(crossprod(matrix(c(1L, NA, 3L, 4L), 2))); print(tcrossprod(matrix(c(1L, NA, 3L, 4L), 2)))
l <- matrix(c(TRUE, FALSE, TRUE, TRUE, NA, FALSE), 3)
print(crossprod(l)); print(tcrossprod(l)); print(crossprod(l[, 1:2] & TRUE))
# Zero extent.
print(crossprod(matrix(0, 0, 3))); print(tcrossprod(matrix(0, 3, 0)))
print(crossprod(matrix(0, 3, 0))); print(tcrossprod(matrix(0, 0, 3)))
print(crossprod(matrix(numeric(0), 0, 2), matrix(numeric(0), 0, 4)))
print(crossprod(numeric(0)))
# Complex: no symmetric kernel in R (zgemm for both triangles).
cz <- matrix(complex(real = 1:6, imaginary = c(-1, 2, 0.5, -3, 1, 2)), 3)
print(crossprod(cz)); print(tcrossprod(cz)); print(identical(crossprod(cz), crossprod(cz, cz)))
print(crossprod(matrix(c(1+1i, NA, 2-1i, 3+0i), 2)))
print(tcrossprod(matrix(c(1+1i, Inf+0i, 2-1i, 3+0i), 2)))
# Errors.
r <- try(crossprod("a"), silent = TRUE); cat(r)
r <- try(crossprod(matrix(1:6, 3), matrix(1:4, 2)), silent = TRUE); cat(r)
r <- try(tcrossprod(matrix(1:6, 3), matrix(1:4, 2)), silent = TRUE); cat(r)
