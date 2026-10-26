# pmin/pmax on doubles: NA/NaN rules, na.rm, recycling
x <- c(1, NA, 3, NaN, 5, -Inf); y <- c(2, 2, NA, 4, NaN, 0)
pmin(x, y); pmax(x, y); pmin(x, y, na.rm = TRUE); pmax(x, y, na.rm = TRUE); pmin(x, 2.5); pmax(1:6, x); pmin(x, c(0, 10)); pmax(x, y, 3)
pmin(c(a = 1, b = 5), 3); pmax(matrix(1:4, 2), 2.5); pmin(1:6 + 0.5, 4L); pmax(numeric(0), 1); pmin(c(1, 2), c(0, 5, 1, 9))
