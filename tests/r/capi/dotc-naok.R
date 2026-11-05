# .C/.Fortran refuse NA and non-finite arguments unless NAOK = TRUE
# (dotcode.c do_dotCode, naokfind); errors name the calling closure.
r <- function(expr) tryCatch(expr, error = function(e) print(e), warning = function(w) print(w))
r(qr(c(1, NA)))
r(qr(c(1, Inf)))
r(qr(c(1, NaN)))
f <- function(t, ldt) .Fortran(base:::.F_dtrco, t, ldt, 1L, 0, 0, 1L)
r(f(1, NA_integer_))
r(f(NA_real_, 1L))
r(f(complex(real = NA, imaginary = 0), 1L))
r(f(1, NA))
g <- function(x) .Fortran(base:::.F_dqrdc2, qr = x, 2L, 2L, 1L, 1e-7, rank = 1L, qraux = double(1), pivot = 1:1, double(2), NAOK = TRUE)$rank
g(matrix(c(1, NaN), 2))
h <- function(x) .Fortran(base:::.F_dqrdc2, qr = x, 2L, 2L, 1L, 1e-7, rank = 1L, qraux = double(1), pivot = 1:1, double(2), NAOK = NA)$rank
r(h(matrix(c(1, NaN), 2)))
k <- function(x) .Fortran(base:::.F_dqrdc2, qr = x, 2L, 2L, 1L, 1e-7, rank = 1L, qraux = double(1), pivot = 1:1, double(2), NAOK = FALSE)$rank
r(k(matrix(c(1, NaN), 2)))
r(.Fortran(base:::.F_dqrdc2, qr = matrix(c(1, 2), 2), 2L, 2L, 1L, 1e-7, rank = 1L, qraux = double(1), pivot = 1:1, double(2), NAOK = TRUE, NAOK = TRUE)$rank)
