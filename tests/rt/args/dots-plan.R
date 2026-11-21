# A compiled call whose callee's `...` formal takes some of its arguments,
# called again from the same site: the later calls are bound by the site's
# plan (X9-5), the first by the matcher. Each line runs three times.
f <- function(a, ..., z = "z") list(n = ...length(), names = ...names(), a = a, z = z)
g <- function() for (i in 1:3) print(f(1, 2, b = 3, z = 4))
g()
h <- function(x, ...) ..2
k <- function() for (i in 1:3) print(h(0, "one", "two"))
k()
# an empty argument into `...`, and missing()
m <- function(...) c(...length(), missing(..1))
k2 <- function() for (i in 1:3) print(tryCatch(m(, 2), error = conditionMessage))
k2()
# laziness: an argument in `...` is forced only when used
cnt <- 0
side <- function() { cnt <<- cnt + 1; cnt }
lz <- function(a, ...) if (a) ..1 else "unused"
k3 <- function() for (i in 1:3) print(c(lz(FALSE, side()), lz(TRUE, side()), cnt))
k3()
# the expressions, the call and match.call (promargs)
sb <- function(...) substitute(list(...))
mc <- function(a, ...) list(sys.call(), match.call())
k4 <- function() for (i in 1:3) { print(sb(x + 1, y = 2)); print(mc(1, q = 2, 3)) }
k4()
# passed on: the callee hands its `...` to another
inner <- function(...) sum(...)
outer <- function(...) inner(...)
k5 <- function() for (i in 1:3) print(outer(1, 2, 3))
k5()
