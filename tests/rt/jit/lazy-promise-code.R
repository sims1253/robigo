# The promises compiled code makes hold MAKEPROM's byte code, translated by
# robigo-rt at its first force (GNU R's bcEval runs it only then). Nothing of
# that shows: values, the call an error inside the promise reports, the
# expression substitute() and a deparse give, a force from C (get, eval),
# and the bytes of a promise never forced.
g <- function(y) y
f <- compiler::cmpfun(function(a, b) { x <- a + 1; g(x * b + a) })
print(c(f(1, 2), f(3, 4)))
# an error inside the promise: its message and call, and the calls on the stack
e <- compiler::cmpfun(function(a) g(a + "z"))
r <- tryCatch(e(1), error = function(c) c)
print(conditionMessage(r)); print(conditionCall(r))
w <- function(y) { y; NULL }
s <- compiler::cmpfun(function(a) w(sys.call()))
print(s(1))
calls <- compiler::cmpfun(function(a) g(length(sys.calls())))
print(calls(1) > 0)
# substitute() and deparse of the promise
sb <- function(y) substitute(y)
q <- compiler::cmpfun(function(a) sb(a + 2 * a))
print(q(1)); print(deparse(q(1)))
# forced from C: get() and eval() of the variable in the callee's frame
gf <- function(y) get("y")
ev <- function(y) eval(quote(y))
p <- compiler::cmpfun(function(a) c(gf(a * 10), ev(a * 100)))
print(p(2))
# a promise never forced: its environment serialized, the promise in it
ne <- function(y) environment()
n <- compiler::cmpfun(function(a) ne(a + 1))
env <- n(5)
x <- serialize(env, NULL)
print(length(x))
print(paste(as.character(x), collapse = ""))
print(eval(quote(y), unserialize(x)))
