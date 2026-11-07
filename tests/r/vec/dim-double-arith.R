# Arithmetic on double arrays whose only attribute is `dim` in compiled
# code (vm::slow::pure_plan): conformable arrays, or an array with a vector
# of its length or a scalar, keep the extents (R: arithmetic.c:R_binary);
# every other shape takes the builtin with its warnings and errors.
f <- function() {
  m <- matrix(c(1, 2, 3, 4, 5, 6), 2); k <- matrix(c(6, 5, 4, 3, 2, 1), 2)
  print(m + k); print(m - k); print(m * k); print(m / k); print(m ^ 2); print(k %/% m)
  print(0.01 / (m * m * m)); print(2 * m); print(m - 1); print(1 - m)
  print(m * c(1, 2, 3, 4, 5, 6)); print(c(6, 5, 4, 3, 2, 1) / m)
  print(m + c(10, 20)); print(m * 1:6); print(m + 1L)
  print(tryCatch(m + matrix(1, 3, 2), error = function(e) conditionMessage(e)))
  print(tryCatch(m + 1:12, error = function(e) conditionMessage(e)))
  print(matrix(5) + c(1, 2, 3))
  print(matrix(5) * 2); print(array(1:8 / 2, c(2, 2, 2)) * array(8:1, c(2, 2, 2)))
  a <- array((1:24) / 4, c(2, 3, 4)); print(a * a - a)
  n <- matrix(c(1, 2), 1, dimnames = list("r", c("a", "b"))); print(n * 2)
  e <- matrix(numeric(0), 0, 3); print(e + 1); print(dim(e * e))
  x <- matrix(c(NA, NaN, 1, NA), 2); y <- matrix(c(NaN, NA, NA, 2), 2)
  print(x + y); print(y * x); print(x - NaN); print(NA_real_ / x)
  b <- m; b <- b * 3; print(m); print(b)
  acc <- matrix(0, 2, 3); for (i in 1:20) acc <- acc + 0.01 * m; print(acc)
  r <- m; for (i in 1:5) r <- r * 1.5 + k; print(r)
  print(structure(1:4, tsp = c(1, 4, 1)) * 2); print(structure(1:4 / 2, dim = 4L, class = "foo") * 2)
}
f()
