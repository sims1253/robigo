# Complex `*` and integer `^` over vectors, two elements at a time: the
# results (NaN payloads too) are those of the element-by-element C99 code.
p <- c(0, -0, 1, -2.5, 1e-310, 1e308, Inf, -Inf, NaN, NA)
z <- as.vector(outer(p, p, function(a, b) complex(real = a, imaginary = b)))
show <- function(v) print(paste(format(Re(v)), format(Im(v)), is.na(v), is.nan(Re(v)), is.nan(Im(v))))
w <- rev(z)
show(z * w)
show(w * z)
show(z * z)
show(z[-1] * w[-1])
for (k in c(2, 3, 5, 8)) show(z ^ k)
show(z[1:7] ^ 2L)
show(z ^ 2.5)
# Mandelbrot iterations escape through Inf + NaN i.
c0 <- complex(real = seq(-2, 1, length.out = 9), imaginary = 0.7)
v <- rep(0+0i, 9)
for (i in 1:60) v <- v * v + c0
show(v)
V <- 0
for (i in 1:12) V <- V^2 + c0
show(V)
show(exp(-abs(V)))
print(exp(c(-745, -745.2, -746, -800, -Inf, NaN, NA, 0)))
