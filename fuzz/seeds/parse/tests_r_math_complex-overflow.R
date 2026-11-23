# Complex overflow arithmetic: multiplication and power with values
# whose products overflow. GNU R 4.5.0's compiled C99 multiply
# (complex_binary, SET_C99_COMPLEX) recovers infinities only when BOTH
# parts are NaN (Annex G / __muldc3), so (1e308+1e308i)^2 is NaN+Infi,
# not -Inf+Infi: another build's compiler may expand the recalc
# differently. Recorded from GNU R 4.5.0.
options(digits = 15)
z <- 1e308 + 1e308i
print(z * z)
print(z^2)
print(z^1.5)
print(z^0.5)
print(z^-2)
print(z * z * z)
print((1e308+0i) * (1e308+0i))
print((0+1e308i) * (0+1e308i))
print((1e308+0i) * (0+1e308i))
print((Inf+1i)^2)
print((Inf+NaN*1i)^2)
print(z / (1e308 - 1e308i))
print(z / 1e300i)
print(exp(100+100i))
print(log(Inf+0i))
print(sqrt(z))
print(try(polyroot(c(z, -2, 0, 1)), silent = TRUE))
v <- c(z, 1i, 0i, NA)
print(v * v)
print(v^2)
print(sum(v * v))
print(crossprod(v, v))
m <- matrix(c(1e308, 1e308, -1e308, 1e308), 2) + 0i
print(m %*% m)
print((NaN+1i) * (Inf+1i))
print((Inf+1i) * (NaN+1i))
print((NA+1i) * (Inf+1i))
print((Inf+1i) * (Inf+1i))
