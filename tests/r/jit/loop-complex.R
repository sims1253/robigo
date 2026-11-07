# Complex scalars in native loops: + - * with complex, double and integer
# operands, abs(), constants; NaN results and NA operands leave native code
# (the VM computes their payloads and __muldc3's recovery).
show <- function(z) print(sprintf("%s %s %s", format(Re(z), digits = 17), format(Im(z), digits = 17), is.na(z)))
mb <- function(n, lim = 2, iter = 50) {
  out <- integer(0)
  for (y in 0:(n - 1)) {
    bits <- 0L
    x <- 0L
    while (x < n) {
      c <- 2 * x / n - 1.5 + 1i * (2 * y / n - 1)
      z <- 0 + 0i
      i <- 0L
      while (i < iter && abs(z) <= lim) {
        z <- z * z + c
        i <- i + 1L
      }
      bits <- 2L * bits + as.integer(abs(z) <= lim)
      if ((x <- x + 1L) %% 8L == 0) {
        out <- c(out, bits)
        bits <- 0L
      }
    }
  }
  out
}
for (k in 1:5) r <- mb(16)
print(r)
print(mb(9))
esc <- function(c, n) { z <- 0i; for (i in 1:n) z <- z * z + c; z }
for (k in 1:20) esc(0.1 + 0.1i, 30)
show(esc(0.1 + 0.1i, 30))
show(esc(1 + 1i, 5))
show(esc(1 + 1i, 12))      # overflows to Inf, then NaN parts
show(esc(1 + 1i, 40))
show(esc(complex(real = NA, imaginary = 0), 3))
show(esc(complex(real = NaN, imaginary = 1), 3))
mix <- function(n, a, b) {
  z <- 1i; s <- 0
  for (i in 1:n) {
    z <- z * a + b - i
    z <- -z + 2L * z
    s <- s + abs(z)
  }
  list(z, s)
}
for (k in 1:20) mix(10, 0.5, 1)
r <- mix(10, 0.5, 1); show(r[[1]]); print(r[[2]], digits = 17)
r <- mix(10, 2i, 1L); show(r[[1]]); print(r[[2]], digits = 17)
r <- mix(10, 0.5, NA_integer_); show(r[[1]]); print(r[[2]])
r <- mix(3, NA_real_, 1); show(r[[1]]); print(r[[2]])
w <- function(n) { z <- 0i; for (i in 1:n) z <- z + 1e308 * (1 + 1i); z }
for (k in 1:20) w(1)
show(w(1)); show(w(2)); show(w(3))
# A complex variable assigned before it is read; its type at the end.
t1 <- function(n) { for (i in 1:n) { q <- i * (1 + 2i); q <- q - 1 }; q }
for (k in 1:20) t1(3)
show(t1(7)); print(typeof(t1(7)))
