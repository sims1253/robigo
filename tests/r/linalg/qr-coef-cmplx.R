# R: Lapack.c:qr_coef_cmplx — a complex qr$qr makes qr.coef take the LAPACK
# complex path (qr.R checks is.complex(qr$qr) first), solving Q' B with
# zunmqr and then the triangular system with ztrtrs.
A <- matrix(c(1, 2, 3, 4, 5, 2, 1, 4, 3, 6, 5, 1, 2, 8, 1, 3, 3, 1, 2, 4), 5, 4)
B <- matrix(complex(real = c(1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
                    imaginary = c(0.5, -1, 2, -2.5, 1, -0.5, 1.5, -1.5, 3, -3)), 5, 2)
# A LAPACK factor, made complex so that qr.coef takes the complex branch;
# this is the shape La_qr_cmplx (zgeqp3) produces.
q <- qr(A, LAPACK = TRUE)
q$qr <- q$qr + 0i
q$qraux <- q$qraux + 0i
cat(is.complex(q$qr), q$rank, "\n")
print(round(qr.coef(q, B), 4))

# The coefficients solve the normal equations.
print(round(crossprod(A) %*% qr.coef(q, B), 4))
print(round(crossprod(A, B), 4))

# A single-column complex right-hand side.
y <- complex(real = 1:5, imaginary = 5:1)
print(round(qr.coef(q, y), 4))

# A numeric right-hand side is coerced to complex; a bare vector is not a
# matrix, so the isMatrix check fires before any coercion, with the same
# "complex matrix" wording.
print(round(qr.coef(q, cbind(1:5)), 4))
cat(tryCatch(qr.coef(q, 1:5), error = conditionMessage), "\n")

cat(q$pivot, "\n")
