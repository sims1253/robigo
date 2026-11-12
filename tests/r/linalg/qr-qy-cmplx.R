# R: Lapack.c:La_qr_cmplx/qr_qy_cmplx — complex QR with qr.qy/qr.qty,
# qr.Q/qr.R/qr.X and qr.solve, plus pivoted dimnames.
A <- matrix(complex(real = c(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
                    imaginary = c(0.5, -1, 2, -2.5, 1, -0.5, 1.5, -1.5, 3, -3, 4, -4)), 4, 3,
            dimnames = list(letters[1:4], c(x = "p", y = "q", z = "r")))
q <- qr(A)
print(dimnames(q$qr))
print(q$pivot)
print(round(qr.Q(q), 6))
print(round(qr.R(q), 6))
print(round(qr.X(q), 6))
y <- cbind(complex(real = 1:4, imaginary = c(1, 0, -1, 2)))
# GNU R's qr_qy_cmplx overwrites a complex matrix argument in place, so
# each call gets a fresh copy (y + 0) here.
print(round(qr.qy(q, y + 0), 6))
print(round(qr.qty(q, y + 0), 6))
print(round(qr.qy(q, qr.qty(q, y + 0)), 6))
print(round(qr.coef(q, y + 0), 6))
x <- cbind(as.complex(1:11), as.complex(1))
print(round(qr.solve(x, as.complex(2 * (1:11) + 3)), 8))
print(round(solve(qr(matrix(complex(real = c(2, 1, 1, 3), imaginary = c(1, 0, 0, -1)), 2))), 8))
print(tryCatch(.Internal(qr_qy_cmplx(q, 1:4, FALSE)), error = conditionMessage))
print(tryCatch(.Internal(qr_qy_cmplx(q, y, NA)), error = conditionMessage))
print(tryCatch(.Internal(qr_qy_cmplx(q, y[1:3, , drop = FALSE], TRUE)), error = conditionMessage))
print(tryCatch(.Internal(La_qr_cmplx(1:4)), error = conditionMessage))
