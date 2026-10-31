# polyroot(): complex.c:do_polyroot and R_cpolyroot (TOMS 419)
op <- function(x) print(x, digits = 17)

# Exact roots of low-degree polynomials, coefficients in increasing powers.
# (x - 1)(x - 2) = 2 - 3x + x^2
op(polyroot(c(2, -3, 1)))
# x^2 + 1
op(polyroot(c(1, 0, 1)))
# x^3 - 1 (the roots of unity)
op(polyroot(c(-1, 0, 0, 1)))
# (x + 1)^8: a repeated root
op(polyroot(c(1, 8, 28, 56, 70, 56, 28, 8, 1)))
# (1 + x)^2 with trailing zero coefficients: they are dropped
op(polyroot(c(1, 2, 1, 0, 0, 0)))

# Integer, logical and complex coefficients.
op(polyroot(1:2))
op(polyroot(FALSE))
op(polyroot(c(TRUE, TRUE, TRUE)))
op(polyroot(c(1 + 1i, -2i, 3 - 1i)))
op(polyroot(c(0, 0, 1)))
op(polyroot(c(0, 0, 0, 1 + 0i)))

# Empty and constant polynomials have no zeros.
op(polyroot(numeric(0)))
op(polyroot(complex(0)))
op(polyroot(0))
op(polyroot(0 + 0i))

# Named and dimensioned coefficients: attributes are not kept.
x <- c(a = 1, b = 0, c = 1)
op(polyroot(x))
op(polyroot(matrix(c(1, 0, 1, 0), 2)))

# A real coefficient vector with no exact roots. Its near-zero imaginary
# parts depend on `hypot`'s last bit, which differs between glibc versions
# (the oracle records Debian 13's), so round them away.
print(round(polyroot(c(1, 0.0853462951557329, -0.433003162033324,
                       0.141816558560935, -0.268523717394886, -0.0970671649038473)), 12))
op(polyroot(c(1, -1.16348488318732, 0.667550726251972, -0.342308178637008)))

# Scaling: very large and very small coefficients (cpoly_scale).
op(polyroot(c(1e300, 1, 1)))
op(polyroot(c(1e-300, 1, 1)))
# A polynomial the root finder cannot deflate: R reports the failure.
print(tryCatch(polyroot(c(1e-200, 2e200, 1)), error = function(e) conditionMessage(e)))

# Invalid coefficients and types.
for (z in list(c(NA, 1), c(NaN, 1, 1), c(Inf, 1), c(1, -Inf, 1), c(1, 1, NA_complex_))) {
    print(tryCatch(polyroot(z), error = function(e) conditionMessage(e)))
}
print(tryCatch(polyroot("x"), error = function(e) conditionMessage(e)))
print(tryCatch(polyroot(NULL), error = function(e) conditionMessage(e)))
print(tryCatch(polyroot(list(1, 2)), error = function(e) conditionMessage(e)))
