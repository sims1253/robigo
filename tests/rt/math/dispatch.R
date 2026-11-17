# the Math group's methods: do_math1, trunc, log, log2, log10
Math.unit <- function(x, ...) {
    cat(.Generic, ": ", nargs(), " argument(s)\n", sep = "")
    structure(get(.Generic)(unclass(x), ...), class = "unit")
}
print.unit <- function(x, ...) cat("<unit>", unclass(x), "\n")
u <- structure(c(1.5, 4, 100), class = "unit")
floor(u)
sqrt(u)
gamma(u)
tanpi(u)
trunc(u)
log(u)
log(u, 10)
log(u, base = 2)
log(x = u)
log2(u)
log10(u)
exp(u)
n <- structure(c(a = 1.5, b = -2.5), class = "nomethod")
floor(n)
log(n)
log(n, 2)
log2(n)
# log2 and log10 dispatch again on log(x, base): a `.default` alias skips
# only the first dispatch
log2.a <- function(x, ...) { print(sys.call()); print(nargs()); "method" }
log2.default <- .Primitive("log2")
log2.default(structure(8, class = "a"))
Math.b <- function(x, ...) { print(sys.call()); cat(.Generic, nargs(), "\n"); "group" }
log10.default <- .Primitive("log10")
log10.default(structure(100, class = "b"))
log10.default(structure(100, class = "nomethod"))
