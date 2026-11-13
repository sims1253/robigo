# The kind of each argument binding before and after its first read, as
# .Internal(getBindingType()) reports it (docs/g3.md §6.2-6.3, E5): a
# constant, a symbol and a call supplied, a default, a missing argument,
# `...`; through `...` forwarding (GNU R's double promise); after
# assignment, rm and delayedAssign. The default JIT is on: a closure that
# calls `f` is byte-compiled by its second call, and a constant argument
# is then a value (PUSHCONSTARG).
bt <- function(s, e) .Internal(getBindingType(s, e))
f <- function(a, b, cc, d = 1, e, ...) {
  env <- environment()
  r <- c(bt(quote(a), env), bt(quote(b), env), bt(quote(cc), env),
         bt(quote(d), env), bt(quote(e), env), bt(quote(...), env))
  a; b; cc; d
  c(r, "|", bt(quote(a), env), bt(quote(b), env), bt(quote(cc), env),
    bt(quote(d), env), bt(quote(e), env))
}
y <- 2
print(f(1, y, y + 1))
print(f(1, y, y + 1, , , 5))
# the first call of w runs its body unevaluated by the JIT, later ones
# compiled
w <- function() f(1, y, y + 1)
print(w()); print(w()); print(w())
h <- function(x, y) {
  env <- environment()
  r1 <- c(bt(quote(x), env), bt(quote(y), env))
  x
  c(r1, bt(quote(x), env), bt(quote(y), env))
}
g <- function(...) h(...)
print(g(1, y))
k <- function(...) { ..1; h(...) }
print(k(y, y))
m <- function(x) { x <- 3; bt(quote(x), environment()) }
print(m(1))
n <- function(x) { rm(x); bt(quote(x), environment()) }
print(n(1))
p <- function(x = stop("no")) bt(quote(x), environment())
print(p())
q <- function(x) {
  delayedAssign("z", x + 1)
  c(bt(quote(z), environment()), bt(quote(x), environment()))
}
print(q(y))
# a function call looks `c` up from the frame: the argument named c is
# forced by findFun
s <- function(c, kk) {
  env <- environment()
  r <- bt(quote(kk), env)
  c(r, bt(quote(c), env), bt(quote(kk), env))
}
print(s(y, y))
# other kinds and the errors
print(bt(quote(nope), globalenv()))
delayedAssign("dz", 1 + 1)
print(bt(quote(dz), globalenv()))
invisible(dz)
print(bt(quote(dz), globalenv()))
makeActiveBinding("ab", function() 1, globalenv())
print(bt(quote(ab), globalenv()))
print(bt(quote(pi), baseenv()))
try(bt(1, globalenv()))
try(bt(quote(x), 1))
