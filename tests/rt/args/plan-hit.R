# Calls bound by their site's match plan: from the second call on, a call
# site of compiled code binds its callee's formals as the first call's
# match did, and what reads the call's argument list later (UseMethod,
# NextMethod, match.call, sys.call) sees GNU R's list. Each case calls from
# a loop, so the sites are compiled and their plans used.

# defaults, an empty argument, named arguments out of order, a `...` formal
# that gobbles nothing
f <- function(x, y = x * 10, ..., z = "z") {
  cat(" x", x, "y", y, "z", z, "missing(y)", missing(y), "nargs", nargs(), "dots", ...length(), "\n")
}
g <- function() for (i in 1:3) {
  f(i)
  f(i, )
  f(z = "named", i)
  f(y = 2, x = i)
}
g()

# a constant argument is not modified by the callee's write
k <- function(v) { v[1] <- 99; v }
g <- function() for (i in 1:3) cat(" k", k(c(1, 2, 3)), k(5L), "\n")
g()

# substitute() and missing() of a supplied argument and of a default
s <- function(a, b = a + 1) cat(" substitute", deparse(substitute(a)), deparse(substitute(b)), missing(b), "\n")
g <- function() for (i in 1:3) { s(i + 1); s(i * 2, b = i) }
g()

# match.call() and sys.call() at a site bound by its plan
mc <- function(alpha, beta = 2, ...) print(list(match.call(), sys.call()))
g <- function() for (i in 1:2) { mc(i); mc(beta = i, 3); mc(i, , 4) }
g()

# UseMethod from a site bound by its plan: the dispatch object, named
# arguments out of order, an empty argument, a forced argument
area <- function(shape, scale = 1, unit) UseMethod("area")
area.sq <- function(shape, scale = 1, unit) {
  cat(" sq side", shape$side, "scale", scale, "missing(unit)", missing(unit), "\n")
  shape$side^2 * scale
}
area.default <- function(shape, scale = 1, unit) 0
sq <- structure(list(side = 2), class = "sq")
g <- function() for (i in 1:3) {
  print(area(sq))
  print(area(scale = i, sq))
  print(area(sq, , "cm"))
  print(area(i))
}
g()

# a generic that forces its argument, and NextMethod
desc <- function(x, label = "x") { force(label); UseMethod("desc") }
desc.b <- function(x, label = "x") { cat(" b:", label, "\n"); NextMethod() }
desc.a <- function(x, label = "x") cat(" a:", label, class(x), "\n")
desc.default <- function(x, label = "x") cat(" default:", label, "\n")
o <- structure(1, class = c("b", "a"))
g <- function() for (i in 1:3) { desc(o, paste0("l", i)); desc(label = "named", o); desc(i) }
g()

# one site, callees of different formals in turn
run <- function(h, v) h(v)
f1 <- function(x) x + 1
f2 <- function(y, x = 5) y * x
f3 <- function(...) nargs()
f4 <- function(value, partial = TRUE) value
g <- function() for (i in 1:3) cat(" run", run(f1, i), run(f2, i), run(f3, i), run(f4, i), "\n")
g()

# the site's plan replaced while its call runs, then the call's argument
# list read: by UseMethod, match.call() and sys.call()
gen <- function(obj, n = 0) {
  run(function(a, b = 2) a + b, n)
  UseMethod("gen")
}
gen.p <- function(obj, n = 0) cat(" gen.p n", n, "\n")
mc2 <- function(first, second = "s") {
  run(function(...) NULL, 1)
  print(match.call())
  print(sys.call())
}
p <- structure(list(), class = "p")
g <- function() for (i in 1:2) { run(function(o) gen(o, i), p); run(mc2, i) }
g()

# a site whose callee is replaced by closures of other formals: matched
# again
h <- function(x) x
g <- function() for (i in 1:3) {
  cat(" h", run(h, i), "\n")
  if (i == 1) h <<- function(x, y = 10) x
  if (i == 2) h <<- function(y, x = 10) x + y
}
g()

# partial matching: matched again at every call, warned under the option
pm <- function(value, verbose = FALSE) value
g <- function() for (i in 1:2) print(pm(val = i))
g()
op <- options(warnPartialMatchArgs = TRUE)
g()
options(op)

# an unused argument errs at every call
u <- function(x) x
g <- function(i) u(i, extra = 1)
for (i in 1:2) print(tryCatch(g(i), error = conditionMessage))
