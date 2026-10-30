# eigen(): symmetric, non-symmetric (real and complex spectra), complex
# input, only.values — the La_rs/La_rg/La_rs_cmplx/La_rg_cmplx internals.
options(digits = 17)

## symmetric 2x2 with nice values
m <- matrix(c(2, 1, 1, 2), 2, 2)
e <- eigen(m)
print(e$values)
print(e$vectors)

## Hilbert 5x5: clustered positive eigenvalues
i <- 1:5
h <- 1 / outer(i - 1, i, "+")
eh <- eigen(h)
print(eh$values)
print(colSums(eh$vectors^2))
print(eigen(h, only.values = TRUE)$values)

## non-symmetric, real spectrum
a <- matrix(c(4, 1, 2, 0, 3, -1, 0, 1, 1), 3, 3)
ea <- eigen(a)
print(ea$values)
print(ea$vectors)

## non-symmetric with a complex-conjugate pair
b <- matrix(c(1, -1, 1, 1), 2, 2)
eb <- eigen(b)
print(eb$values)
print(eb$vectors)
print(eigen(b, only.values = TRUE)$values)

## integer input is coerced; eigenvalues of a 2x2 with a repeated root
print(eigen(matrix(1:4, 2, 2), symmetric = FALSE, only.values = TRUE)$values)

## complex Hermitian
zc <- matrix(c(1+0i, 1i, -1i, 2+0i), 2, 2)
print(eigen(zc)$values)
print(eigen(zc)$vectors)

## complex general, only values
zg <- matrix(c(1+2i, 3, 1i, 2-1i, 0, 1, 2, 1i, 1), 3, 3)
print(eigen(zg, only.values = TRUE)$values)
