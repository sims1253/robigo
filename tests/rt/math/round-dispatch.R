# round() and signif() dispatch on the "Math" group: the call the method
# sees is LCONS(CAR(call), args), the matched values; round's method sees
# the arguments past x
Math.foo <- function(x, ...) {
    print(sys.call()); print(.Generic); print(list(...)); print(nargs())
    print(substitute(x)); "foo-method"
}
x <- structure(1.234, class = "foo")
round(x)
round(x, 2)
round(x, digits = 2)
round(x, 2, extra = 3)
round(digits = 2, x = x)
signif(x)
signif(x, 2)
signif(digits = 3, x)
h <- function(v) round(v); h(x)
k <- function(...) round(...); k(2.5); k(2.567, 1); k(x, 2)
round.bar <- function(x, digits = 0, ...) { cat("round.bar", digits, "\n"); NextMethod() }
y <- structure(c(1.2345, 2.3456), class = "bar")
round(y, 2)
round(y)
signif.bar <- function(x, digits = 6) { cat("signif.bar\n"); unclass(x) }
signif(y, 2)
round(1.5, structure(1, class = "foo"))
round(as.Date("2024-01-15") + 0.6)
d <- as.difftime(c(1.234, 5.678), units = "hours"); round(d, 1)
p <- as.POSIXct("2024-01-15 10:31:45", tz = "UTC"); round(p, "mins"); round(p, units = "hours")
round(data.frame(a = c(1.23, 4.56), b = c(7.891, 0.12)), 1)
