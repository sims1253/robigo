# R: Lapack.c:La_qr_cmplx (zgeqp3) — qr.default sends any complex matrix to
# .Internal(La_qr_cmplx).
A <- matrix(complex(real = c(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
                    imaginary = c(0.5, -1, 2, -2.5, 1, -0.5, 1.5, -1.5, 3, -3, 4, -4)), 4, 3)
q <- qr(A)
cat(is.complex(q$qr), q$rank, "\n")
print(round(q$qr, 6))
print(round(q$qraux, 6))
print(q$pivot)
print(round(qr.coef(q, complex(real = 1:4, imaginary = 4:1)), 6))
