# Complex scalar arithmetic and abs() in compiled code (closure bodies and
# loops): the same values as the builtins, operand types mixed.
hex <- function(z) cat(sprintf("%a %a", Re(z), Im(z)), "\n")
ops <- function(a, b) list(a + b, a - b, a * b, a / b, a ^ b)
show <- function(a, b) for (v in ops(a, b)) hex(v)
show(1+2i, 3-1i)
show(1+2i, 2)
show(2, 1+2i)
show(1+2i, 3L)
show(TRUE, 1+2i)
show(0+0i, 0+0i)
show(1+1i, 0+0i)
show(complex(real = Inf, imaginary = 1), 2+3i)
show(complex(real = Inf, imaginary = NaN), 0+1i)
show(complex(real = NaN, imaginary = 1), 1+1i)
show(NA_complex_, 1+1i)
show(1+1i, NA_real_)
show(1+1i, NA_integer_)
show(1e308+1e308i, 1e308+1e308i)
show(-0+0i, -0-0i)
f <- function(z) abs(z)
for (z in list(3+4i, -0-0i, complex(real = Inf, imaginary = NaN), NA_complex_, 1e308+1e308i)) print(f(z))
# Mandelbrot-style iteration in a loop.
mb <- function(c, iter, lim) {
  z <- 0+0i
  i <- 0L
  while (i < iter && abs(z) <= lim) {
    z <- z * z + c
    i <- i + 1L
  }
  c(i, Re(z), Im(z))
}
print(mb(-0.5+0.5i, 50L, 2))
print(mb(0.3+0.6i, 50L, 2))
print(mb(-2+0i, 50L, 2))
# Attributes and vectors keep the builtins' behaviour.
g <- function(a, b) a * b + a
print(g(c(x = 1+1i), 2+0i))
print(g(c(1+1i, 2-1i), 1i))
print(g(structure(1+1i, foo = "bar"), 2))
h <- function(a, b) a %% b
print(tryCatch(h(1+1i, 2), error = conditionMessage))
print(tryCatch(h(1+1i, 2i), error = conditionMessage))
