# GNU R's Lapack.c:qr_qy_cmplx calls coerceVector(Bin, CPLXSXP), which hands
# back the complex argument itself, so zunmqr overwrites the caller's matrix:
# after qr.qy(q, y), y holds the result. robigo copies on modify and leaves
# y alone.
A <- matrix(complex(real = c(1, 2, 3, 4), imaginary = c(1, 0, -1, 2)), 2, 2)
q <- qr(A)
y <- matrix(complex(real = 1:2, imaginary = 0), 2, 1)
r <- qr.qy(q, y)
print(identical(r, y))
