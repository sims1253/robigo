# R: arithmetic.c:logical_unary — +x and -x on a logical keep only names,
# dim and dimnames; integer, double and complex operands keep everything.
z <- structure(c(TRUE, FALSE, NA), foo = "bar", class = "zz", names = c("a", "b", "c"))
print(attributes(+unclass(z)))
print(attributes(-unclass(z)))
print(attributes(!unclass(z)))
m <- matrix(c(TRUE, FALSE, NA, TRUE), 2, dimnames = list(c("r1", "r2"), c("c1", "c2")))
attr(m, "foo") <- 1
print(+m)
print(attributes(-m))
a1 <- array(c(TRUE, FALSE), 2, dimnames = list(c("p", "q")))
print(attributes(-a1))
print(attributes(-structure(1:2, foo = "bar", class = "zz")))
print(attributes(-structure(c(1.5, 2), foo = "bar")))
# a logical ts loses its class, a numeric one keeps it (via Ops.ts)
x <- stats::ts(c(a = TRUE, b = FALSE, c = NA, d = TRUE), frequency = 4, start = 2000)
x; +x; -x; !x
stopifnot(stats::is.ts(!x), !stats::is.ts(+x), !stats::is.ts(-x))
x <- stats::ts(c(a = 1, b = 2, c = 0, d = 4), frequency = 4, start = 2010)
x; +x; -x; !x
stopifnot(!stats::is.ts(!x), stats::is.ts(+x), stats::is.ts(-x))
