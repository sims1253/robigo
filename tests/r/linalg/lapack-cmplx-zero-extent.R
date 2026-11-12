# Zero-extent and hand-made inputs to the complex LAPACK internals: LAPACK
# rejects LDA = 0 and K > NQ through R's xerbla ("BLAS/LAPACK routine ...
# gave error code"), which must be an R error, not the system xerbla's STOP.
f <- function(e) tryCatch({ r <- e; cat("ok:", class(r)[1L], length(r), "\n") },
                          error = function(c) cat("Error:", conditionMessage(c), "\n"))
for (d in list(c(0, 0), c(0, 2), c(2, 0))) {
    cat("== dim", d, "\n")
    m <- matrix(0, d[1], d[2]); z <- matrix(0i, d[1], d[2])
    f(.Internal(La_dtrcon(m, "O"))); f(.Internal(La_dtrcon3(m, "O", "U")))
    f(.Internal(La_ztrcon(z, "O"))); f(.Internal(La_ztrcon3(z, "O", "U")))
    f(.Internal(La_zgecon(z, "O"))); f(.Internal(La_zlange(z, "O")))
    f(.Internal(La_zlange(z, "M"))); f(.Internal(La_qr_cmplx(z)))
    qq <- structure(list(qr = z, rank = 0L, qraux = complex(min(d)),
                         pivot = seq_len(d[2])), useLAPACK = TRUE, class = "qr")
    for (ny in c(0, 1)) {
        y <- matrix(0i, d[1], ny)
        f(.Internal(qr_coef_cmplx(qq, y)))
        f(.Internal(qr_qy_cmplx(qq, y, FALSE)))
        f(.Internal(qr_qy_cmplx(qq, y, TRUE)))
    }
}
# more reflectors than rows
qq <- structure(list(qr = matrix(1i, 2, 3), rank = 2L, qraux = complex(3),
                     pivot = 1:3), useLAPACK = TRUE, class = "qr")
f(.Internal(qr_qy_cmplx(qq, matrix(0i, 2, 1), FALSE)))
f(.Internal(qr_coef_cmplx(qq, matrix(0i, 2, 1))))
f(rcond(matrix(0i, 0, 0)))
f(rcond(matrix(0, 0, 0), triangular = TRUE))
# pivoted column names
a <- matrix(c(1, 0, 0, 2, 0, 0, 3, 1, 0) + 0i, 3, dimnames = list(NULL, c("a", "b", "c")))
q <- qr(a, LAPACK = TRUE)
q$pivot
colnames(q$qr)
