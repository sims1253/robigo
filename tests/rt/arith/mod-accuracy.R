# arithmetic.c:myfmod warns "probable complete loss of accuracy in modulus"
# when the quotient is finite and past 1/LDBL_EPSILON, once per element:
# from %% on doubles (real_binary), and from R_pow's myfmod(y, 2.) for
# (-Inf)^y; never from %/% (myfloor), integer %% or y == 2. Each warning
# with the call it reports: at top level, in closures run by the AST
# (their first call, or one GNU R never compiles), in compiled closures,
# in a callee of a compiled caller (which robigo-rt may run inline), in a
# compiled function and in a top-level loop.
w <- function(expr) withCallingHandlers(expr, warning = function(w) {
  call <- conditionCall(w)
  cat("  warning:", if (is.null(call)) "NULL" else as.character(call[[1L]]), "|", conditionMessage(w), "\n")
  invokeRestart("muffleWarning")
})
print(w(1e308 %% 1))
print(w(-1e308 %% 3))
print(w(c(1e308, 5, 1e300) %% 1))
print(w(1e308 %% 1L))
print(w(1e308 %% TRUE))
print(w(1e19 %% 1))
print(w(1e18 %% 1))
print(w(Inf %% 1))
print(w(1e308 %/% 1))
print(w(.Machine$integer.max %% 3L))
print(w((-Inf)^1e308))
print(w((-Inf)^c(1e308, 3, 2e300, 2)))
print(w((-Inf)^2e19))
print(w((-Inf)^1e18))
print(w(-Inf^1e308))
f <- function(x, y) x %% y
p <- function(x, y) x^y
mk <- function() function(x, y) x %% y
g <- mk()
print(w(f(1e308, 1)))
print(w(g(1e308, 1)))
for (k in 1:3) r <- w(f(1e308, 1))
print(r)
for (k in 1:3) r <- w(p(-Inf, 1e308))
print(r)
m <- function(x) x %% 1
loop <- function() {
  r <- 0
  for (i in 1:3) r <- r + m(1e308)
  r
}
print(w(loop()))
print(w(loop()))
h <- function() {
  for (i in 1:2) NULL
  c(1e308 %% 1, (-Inf)^1e308, 7 %% 2)
}
print(w(h()))
for (i in 1:2) r <- w(1e308 %% 1)
print(r)
# unhandled: the top level's deferred warnings
1e308 %% 1
f(1e308, 1)
