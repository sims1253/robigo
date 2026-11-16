# Two doubles in inline runs: their arithmetic and comparisons go straight
# to the kernels' double arms, and other operand types take the general
# path. The values are GNU R's: which of NA and NaN an operation keeps, the
# sign of zero, infinities, and integers and logicals against doubles.
# Each case runs three times from a compiled closure.
add <- function(x, y) x + y
sub <- function(x, y) x - y
mul <- function(x, y) x * y
dvd <- function(x, y) x / y
pw <- function(x, y) x ^ y
md <- function(x, y) x %% y
idv <- function(x, y) x %/% y
ar <- function(a, b) c(add(a, b), sub(a, b), mul(a, b), dvd(a, b), pw(a, b), md(a, b), idv(a, b))
r1 <- function() list(ar(NA_real_, NaN), ar(NaN, NA_real_), ar(NA_real_, 1), ar(1, NaN),
                      ar(Inf, -Inf), ar(7.5, 2), ar(5L, 2), ar(2.5, 3L), ar(TRUE, 2.5))
for (k in 1:3) print(r1())
# the sign of zero, seen through 1/x
r2 <- function() c(1 / sub(0, 0), 1 / mul(-1, 0), 1 / add(-0, -0), 1 / add(-0, 0))
for (k in 1:3) print(r2())
lt <- function(x, y) x < y
le <- function(x, y) x <= y
eq <- function(x, y) x == y
ne <- function(x, y) x != y
rel <- function(a, b) c(lt(a, b), le(a, b), eq(a, b), ne(a, b))
r3 <- function() list(rel(NaN, 1), rel(1, NA_real_), rel(-0, 0), rel(Inf, Inf), rel(2L, 2.5),
                      rel(TRUE, 0.5), rel(3, 3L))
for (k in 1:3) print(r3())
# a formal compared with a double constant and branched on, then
# arithmetic returned (fused steps): NaN's error comes from `if`
br <- function(n) if (n < 2) n else br(n - 1) + br(n - 2)
cond <- function(e) list(conditionMessage(e), conditionCall(e))
r4 <- function() c(br(10), br(2.5), br(-Inf), br(1L), br(TRUE))
for (k in 1:3) print(r4())
r5 <- function() tryCatch(br(NaN), error = cond)
for (k in 1:3) print(r5())
r6 <- function() tryCatch(br(NA_real_), error = cond)
for (k in 1:3) print(r6())
# argument code of a variable and a scalar or two variables (argument
# shapes) with double operands
id <- function(x) x
r7 <- function() {
  a <- NA_real_; b <- NaN; z <- -0
  c(id(a + 1), id(b + 1), id(a - b), id(b - a), id(a * 2), id(2 * b), 1 / id(z * 1), 1 / id(0 * z))
}
for (k in 1:3) print(r7())
