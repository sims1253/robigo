# x %/% y on doubles: integer values (the common case), around 2^53, signs,
# zero, infinities, NaN and fractions; scalars, vectors and loops.
show <- function(x) print(sprintf("%.17g", x))
a <- c(7, -7, 7, -7, 0, -0, 1, -1, 6, -6, 9007199254740992, -9007199254740991,
       9007199254740993, 2147483647, -2147483648, 1e15 + 1, 4503599627370497)
b <- c(2, 2, -2, -2, 3, 3, 4, 4, -3, 3, 3, 7, 3, -2, 2, 7, 2)
show(a %/% b)
show(a %/% -b)
show(a %/% 1)
show(a %/% -1)
show(c(5, -5, 0) %/% Inf)
show(c(5, -5, 0) %/% -Inf)
show(c(Inf, -Inf, NaN, NA) %/% 3)
show(c(5, -5, 0) %/% 0)
show(c(7.5, -7.5, 1e300, 2^60, -2^60) %/% c(2, 2, 3, 3, 7))
show(c(1, 2, 3) %/% c(2^54, -2^54, 2^60))
show(1e15 %/% 1e-3)
f <- function(n) {
    s <- 0
    lo <- 1L
    hi <- 15L
    for (i in 1:n) {
        mid <- (lo + hi - 1L) %/% 2
        s <- s + mid + (i * 7) %/% 3 + (-i) %/% 4
    }
    s
}
print(f(1000))
x <- seq(-50, 50, by = 1)
print(sum(x %/% 7) + sum(x %/% -3) + sum((x * 1e10) %/% 3e9))
