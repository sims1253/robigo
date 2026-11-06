# Arithmetic on attribute-free double vectors in compiled code
# (vm::slow::dbl_arith): computed without the call, with R's results,
# NA/NaN payloads and recycling; everything else takes the builtin.
f <- function() {
  a <- c(1, 2, 3); b <- c(4, 5, 6)
  print(a - b); print(a + b); print(a * b); print(a / b); print(a ^ b); print(b %/% a)
  print(a * 2.5); print(2.5 - a); print(a / 0); print(-a / 0); print(0 / (a - a))
  x <- c(NA, NaN, 1, NA, NaN); y <- c(NaN, NA, NA, 2, NaN)
  print(x + y); print(y + x); print(x - y); print(x * y); print(y / x); print(x ^ y); print(y ^ x)
  print(x * 1); print(NA_real_ - x); print(x - NaN)
  print(numeric(0) + a[1]); print(a[1] * numeric(0)); print(numeric(0) - numeric(0))
  print(a + c(1, 2)); print(c(1, 2, 3, 4) * c(10, 20))
  print(a %% 2); print(c(1e300, -1) %% 0.3)
  print(a + 1:3); print(1:3 * a); print(a - TRUE); print(c(a = 1, b = 2) * 2); print(matrix(a, 1) + 1)
  v <- a; w <- v; v <- v + 1; print(w); print(v)
  u <- c(1, 2, 3); for (k in 1:5) u <- u * 1.5 + c(0.25, 0.5, 1); print(u)
  for (k in 1:3) u <- 2 * u - u / 3; print(u)
  q <- c(0.1, 0.2, 0.3); r <- q * q + q; print(sprintf("%.17g", r)); print(sprintf("%.17g", q - q * q))
  s <- q; s <- s * s; print(q); print(s)
  dr <- c(1.5, -2.25, 3); mag <- 0.01; m <- 5.5
  print(sprintf("%.17g", dr * m * mag)); print(sprintf("%.17g", c(1, 1, 1) - dr * m * mag))
}
f()
g <- function(x) x - c(1, 1)
print(g(c(5, 6))); print(g(3)); print(tryCatch(g("a"), error = function(e) conditionMessage(e)))
h <- function() { `+` <- function(e1, e2) "plus"; c(1, 2) + c(3, 4) }
print(h())
# complex scalars take the same path
cx <- function() {
  z <- 0+0i; c0 <- -0.5 + 0.25i
  for (k in 1:30) z <- z * z + c0
  print(z); print(z - 1L); print(2L * z); print(z / 0); print(z ^ 2); print((1+2i) ^ 0.5); print(TRUE + 1i)
  print(complex(real = NA, imaginary = 1) * 2); print(NA_real_ + 1i); print(NaN * 1i); print(NA_integer_ - 1i)
  print(tryCatch(z %% 2, error = function(e) conditionMessage(e)))
  print(tryCatch(z %/% 2, error = function(e) conditionMessage(e)))
  w <- c(1i, 2i); print(w * 2); print(w + c(1, 2))
}
cx()
