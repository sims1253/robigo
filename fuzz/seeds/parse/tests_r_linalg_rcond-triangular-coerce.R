# La_dtrcon/La_dtrcon3 accept any matrix and coerce it to double
# (R: Lapack.c:La_dtrcon, La_dtrcon3: isMatrix, then coerceVector)
rcond(matrix(c("1", "0", "0", "2"), 2), triangular = TRUE)
rcond(matrix(c(TRUE, FALSE, TRUE, TRUE), 2), triangular = TRUE)
rcond(matrix(c("1", "0", "0", "2"), 2), triangular = TRUE, uplo = "L")
r <- try(rcond(matrix(c("1", "x", "0", "2"), 2), triangular = TRUE))
r <- try(.Internal(La_dtrcon(1:4, "O")))
