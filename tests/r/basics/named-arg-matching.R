## Argument matching for calls with tagged arguments and `...`: exact
## tags, positional filling around them, partial matching, errors, and
## what the callee sees (missing(), defaults, ..., sys.call, match.call).
## Calls are made from inside closures and loops so compiled code runs them.
show <- function(x) dput(x)
f <- function(a, bb, ..., z = 3) {
  list(a = if (!missing(a)) a, bb = if (!missing(bb)) bb, dots = list(...),
       z = z, mz = missing(z), nd = ...length())
}
run1 <- function() {
  show(f(1, 2, 3))
  show(f(bb = 1, 2, 3))
  show(f(b = 1, 2, 3))
  show(f(x = 1, 2, 3, z = 9))
  show(f(z = 1))
  show(f())
  show(f(1, , 3))
  show(f(a = 1, bb = 2, z = 4, w = 5))
  show(f(z = 1, z2 = 2, 3))
}
run1()
g <- function(x, y = x * 2) c(x = if (!missing(x)) x else NA, y = y, mx = missing(x), my = missing(y))
run2 <- function() {
  for (i in 1:3) show(g(y = i))
  for (i in 1:3) show(g(y = 1, i))
  for (i in 1:3) show(g(x = i))
  show(g(y = , x = 2))
  print(tryCatch(g(x = 1, x = 2), error = conditionMessage))
  print(tryCatch(g(1, 2, 3), error = conditionMessage))
  print(tryCatch(g(z = 1), error = conditionMessage))
  print(tryCatch(g(1, y = 2, x = 3), error = conditionMessage))
}
run2()
h <- function(..., n) list(..., n = if (!missing(n)) n)
k <- function(abc, ...) list(abc = if (!missing(abc)) abc, d = list(...))
run3 <- function() {
  show(h(1, n = 2, m = 3))
  show(h(n = 1, nn = 2))
  show(h(n2 = 1))
  show(k(ab = 1, 2))
  show(k(abc = 1, ab = 2))
  show(k(x = 1, ab = 2))
  show(k(2, a = 1))
  print(tryCatch(k(ab = 1, a = 2), error = conditionMessage))
}
run3()
## Laziness and order of forcing.
lazy <- function(first, second) { second; first; invisible() }
run4 <- function() {
  lazy(second = cat("second\n"), first = cat("first\n"))
  lazy(cat("one\n"), second = cat("two\n"))
}
run4()
## What the callee sees of its call.
w <- function(a, b) sys.call()
mc <- function(a, b, ...) match.call()
sb <- function(a, b) substitute(b)
na <- function(a, b, c) nargs()
run5 <- function() {
  print(w(b = 1, a = 2))
  print(mc(b = 1, 2, c = 3))
  print(sb(b = x + y, a = 1))
  print(na(c = 1, 2))
  print(na(b = 1))
}
run5()
## Dots passed on, and ..N.
p <- function(...) q(...)
q <- function(a, b, ...) c(a = a, b = b, n = ...length())
d2 <- function(...) ..2
run6 <- function() {
  for (i in 1:2) print(p(b = i, a = 10, 7))
  print(d2(x = 1, y = "two"))
  print(tryCatch(d2(x = 1), error = conditionMessage))
}
run6()
## A formal that is only reachable by its full name after `...`.
after <- function(..., verbose = FALSE) c(n = ...length(), v = verbose)
run7 <- function() {
  print(after(1, 2, verbose = TRUE))
  print(after(1, verb = TRUE))
  print(after(verbose = TRUE))
}
run7()
## Repeated formal names (as.function allows them): a tag matching two
## formals is an error, positional matching is not.
dup <- as.function(alist(x = 1, x = 2, x))
run8 <- function() {
  print(tryCatch(dup(x = 3), error = conditionMessage))
  print(dup(3))
  print(dup())
}
run8()
## A formal matched by tag to an empty argument is still filled by position.
em <- function(x = 10, y = 20) c(x, y)
eh <- function(a, b, ...) list(a = if (!missing(a)) a, b = if (!missing(b)) b, d = list(...))
run9 <- function() {
  print(em(x = , 3))
  print(em(y = , 3))
  show(eh(a = , 1, 2))
  show(eh(b = , 1, 2, 3))
}
run9()
print(em(x = , 3))
