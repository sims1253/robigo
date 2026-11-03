# Whole closures in native code (jit/closure.rs): whatever the fast paths do
# not cover must behave as in the VM, after however many calls made the
# closure hot.
hot <- function(f, ...) {
  a <- list(...)
  n <- length(a)
  if (n == 0) for (i in 1:600) f()
  else if (n == 1) { x <- a[[1]]; for (i in 1:600) f(x) }
  else if (n == 2) { x <- a[[1]]; y <- a[[2]]; for (i in 1:600) f(x, y) }
  else stop("hot")
}

fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)
print(fib(20)); print(fib(20L)); print(fib(TRUE)); print(typeof(fib(20L)))
print(fib(NaN == 1 | TRUE))
r <- tryCatch(fib(NA), error = function(e) conditionMessage(e)); print(r)
r <- tryCatch(fib(NA_real_), error = function(e) print(conditionCall(e))); print(r)
print(fib(c(a = 1)))
print(fib(1:1))
r <- tryCatch(fib(c(1, 2)), error = function(e) conditionMessage(e)); print(r)
r <- tryCatch(fib("a"), error = function(e) conditionMessage(e)); print(r)
# integer overflow: NA with a warning, from the right call
big <- function(n) if (n < 1L) 2147483647L else big(n - 1L) + 1L
r <- withCallingHandlers(big(3L), warning = function(w) {
  print(conditionMessage(w)); print(conditionCall(w)); invokeRestart("muffleWarning")
})
print(r)
for (i in 1:600) big(5L)
print(suppressWarnings(big(2L)))
# the recursion limit
deep <- function(n) if (n == 0) 0 else 1 + deep(n - 1)
hot(deep, 10)
print(deep(200))
r <- tryCatch(deep(1e5), error = function(e) substr(conditionMessage(e), 1, 40)); print(r)
op <- options(expressions = 300)
r <- tryCatch(deep(500), error = function(e) substr(conditionMessage(e), 1, 40)); print(r)
print(deep(50))
options(op)
# rebinding a callee after it was compiled
g <- function(x) x + 1
h <- function(x) g(x) * 2
hot(h, 1)
print(h(1))
g <- function(x) x + 100
print(h(1))
hot(h, 1)
print(h(1))
g <- function(x) { cat("called\n"); x }
print(h(1))
rm(g)
r <- tryCatch(h(1), error = function(e) conditionMessage(e)); print(r)
g <- 5
r <- tryCatch(h(1), error = function(e) conditionMessage(e)); print(r)
rm(g)
# rebinding an operator after compilation
add <- function(a, b) a + b
hot(add, 1, 2)
print(add(1, 2))
`+` <- function(e1, e2) "mine"
print(add(1, 2))
rm(`+`)
print(add(1, 2))
hot(add, 1L, 2L)
print(add(2147483647L, 1L))
# a mutually recursive pair, specialized on types
ev <- function(n) if (n == 0) TRUE else od(n - 1)
od <- function(n) if (n == 0) FALSE else ev(n - 1)
hot(ev, 4)
print(ev(101)); print(ev(100L)); print(od(7))
# results of different types stay in the VM
mix <- function(n) if (n > 0) 1L else 2.5
hot(mix, 1)
print(mix(1)); print(mix(0)); print(typeof(mix(1)))
# local variables, loops, early return
isp <- function(n) {
  if (n < 2) return(FALSE)
  i <- 2
  while (i * i <= n) {
    if (n %% i == 0) return(FALSE)
    i <- i + 1
  }
  TRUE
}
hot(isp, 7)
print(sum(sapply(1:100, isp)))
print(isp(2147483647)); print(isp(97L)); print(isp(1))
r <- tryCatch(isp(NA), error = function(e) conditionMessage(e)); print(r)
tri <- function(n) { s <- 0; for (i in 1:n) s <- s + i; s }
hot(tri, 5)
print(tri(10)); print(tri(0)); print(tri(10L)); print(tri(-2))
sumto <- function(n) { s <- 0L; i <- 0L; repeat { i <- i + 1L; if (i > n) break; if (i %% 2L == 0L) next; s <- s + i }; s }
hot(sumto, 5L)
print(sumto(10L)); print(sumto(100000L) )
# warnings in native bodies
root <- function(x) sqrt(x)
hot(root, 4)
r <- withCallingHandlers(root(-1), warning = function(w) { print(conditionMessage(w)); print(conditionCall(w)); invokeRestart("muffleWarning") })
print(r)
md <- function(a, b) a %% b
hot(md, 5, 3)
print(md(5, 0)); print(md(-5, 3)); print(md(5L, 0L)); print(md(1e300, 3))
# a formal or local that shadows a function
sh <- function(f, n) if (n == 0) 0 else f(n - 1)
sh2 <- function(n) sh(function(k) k + 1, n)
hot(sh2, 2)
print(sh2(2))
lo <- function(n) { fib <- function(k) -k; fib(n) }
hot(lo, 2)
print(lo(2))
# arguments that are not all used are not forced
un <- function(a, b) if (a > 0) a else b
hot(un, 1, 2)
print(un(1, stop("not forced")))
print(un(0, 7))
# a free variable is read when the call happens
K <- 10
fr <- function(n) n + K
hot(fr, 1)
print(fr(1)); K <- 20; print(fr(1)); K <- "a"
r <- tryCatch(fr(1), error = function(e) conditionMessage(e)); print(r)
rm(K)
r <- tryCatch(fr(1), error = function(e) conditionMessage(e)); print(r)
# visibility and tracing
vi <- function(x) invisible(x)
hot(vi, 1)
vi(1)
print(withVisible(vi(1))$visible)
tr <- function(x) x * 2
hot(tr, 2)
.primTrace(tr)
print(tr(2))
.primUntrace(tr)
print(tr(2))
# a closure reassigned to a copy with another body, same name
body(fib) <- quote(if (n < 2) 1 else fib(n - 1) + fib(n - 2))
print(fib(10))
environment(fib) <- new.env()
print(fib(10))
# sys.call and friends are not pure; they stay in the VM
wh <- function(n) if (n == 0) sys.nframe() else wh(n - 1)
hot(wh, 3)
print(wh(3))
