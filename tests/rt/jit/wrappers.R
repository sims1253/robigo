# Compiled code calls the .Internal of base's simple wrappers directly
# (cmp.R's cmpSimpleInternal), behind a guard on base's binding
# (BASEGUARD): the wrapper makes no context, its arguments are the
# internal's, evaluated in the internal's order.
f <- function(x) is.vector(x)
print(f(1)); print(f(1))
e <- function() {
  p <- { cat("p\n"); 1 }
  mo <- { cat("mo\n"); "any" }
  is.vector(mode = mo, x = p)
}
print(e()); print(e())
# a local definition is not inlined
lf <- function(x) { is.vector <- function(x, mode) "local"; is.vector(x) }
print(lf(1)); print(lf(1))
# an error the internal raises with error() reports the wrapper's call
v <- function() vector("nope", 1)
tryCatch(v(), error = function(e) print(conditionCall(e)))
tryCatch(v(), error = function(e) print(conditionCall(e)))
# unused arguments and `...` are not inlined: the call reports them
u <- function(a) typeof(a, 2)
tryCatch(u(1), error = function(e) print(conditionMessage(e)))
tryCatch(u(1), error = function(e) print(conditionMessage(e)))
d <- function(...) typeof(...)
print(d(1L)); print(d(1L))
# a global rebinding after compilation fails the guard: the call is generic
q <- function(x) is.vector(x)
print(q(2)); print(q(2))
is.vector <- function(x, mode = "any") "rebound"
print(q(3))
# a missing argument is the caller's error, with no wrapper in the calls
m <- function(x) typeof(x)
print(m(1)); print(m(1))
m()
