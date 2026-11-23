# Re(), Im(), Mod(), Arg() and Conj(): complex.c:do_cmathfuns
op <- function(x) print(x, digits = 17)

# Non-complex input is coerced to double first, so the result of Re() and
# Conj() is double even for logical and integer arguments.
print(typeof(Re(FALSE)))
print(typeof(Conj(TRUE)))
print(typeof(Im(1L)))
print(typeof(Mod(1L)))
print(typeof(Arg(1L)))
op(Re(c(FALSE, TRUE)))
op(Conj(c(FALSE, TRUE)))
op(Re(1:3))
op(Conj(1:3))
op(Im(1:3))

# Im() of a real number is zero, including NA and NaN; Arg() keeps them.
op(Im(c(1, NA, NaN, 2)))
op(Im(as.double(NA)))
op(Im(NA_real_))
op(Im(NaN))
op(Im(Inf))
op(Arg(c(1, -1, 0, NA, NaN, -Inf)))
op(Mod(c(3, -4, NA, NaN, Inf, -Inf)))
op(Re(c(NA_real_, NaN, Inf, -Inf)))
op(Conj(c(NA_real_, NaN, Inf, -Inf)))

# Every attribute is kept, including ones Re()/Im() do not otherwise use.
x <- c(a = 1, b = 2)
attr(x, "foo") <- "foo"
class(x) <- "myclass"
op(attributes(Re(x)))
op(attributes(Conj(x)))
op(attributes(Im(x)))
op(attributes(Mod(x)))
op(attributes(Arg(x)))

y <- matrix(c(1, 2, 3, 4), 2, dimnames = list(c("r1", "r2"), c("c1", "c2")))
attr(y, "scaled:center") <- -0.5
op(attributes(Re(y)))
op(Re(y))
op(Im(y))
op(Mod(y))

z <- c(1 + 2i, -3 - 4i, NA_complex_, NaN + 1i, Inf + 0i)
op(Re(z))
op(Im(z))
op(Mod(z))
op(Arg(z))
op(Conj(z))
attr(z, "foo") <- 1:2
op(attributes(Conj(z)))
op(attributes(Re(z)))
op(attributes(Mod(z)))

# Names survive all five.
w <- c(a = 1 + 1i, b = 2 - 3i)
op(Re(w))
op(Im(w))
op(Mod(w))
op(Arg(w))
op(Conj(w))
