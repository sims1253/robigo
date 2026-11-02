# Calls nested inside inlinable closure bodies: the callee runs inline
# too when pure, and the semantics of the fallback (a real call) must
# agree byte for byte.
f <- function(n) if (n < 2) n else f(n - 1) + f(n - 2)
cat(f(15), "\n")
# Mutual recursion through inlinable bodies.
g1 <- function(n) if (n <= 0) 0 else 1 + g2(n - 1)
g2 <- function(n) if (n <= 0) 0 else 2 + g1(n - 1)
cat(g1(9), g2(9), "\n")
# A nested call whose target stops being inlinable (a print) falls back
# to real calls everywhere; its output order is the value order.
p <- function(n) if (n <= 0) 0 else n + p(n - 1) + 0 * print(n)
cat(p(4), "\n")
# Lazy arguments: the body never reads its second formal.
h <- function(a, b) a
cat(h(f(6), stop("never evaluated")), "\n")
# ... and one it does read, with the error surfacing verbatim.
r <- tryCatch(h(1, stop("boom")), error = function(e) conditionMessage(e))
cat(r, "\n")
# Errors deep in a nested inline run keep their call and message.
deep <- function(n) if (n == 0) stop("bottom") else deep(n - 1)
r <- tryCatch(deep(25), error = function(e) conditionMessage(e))
cat(r, "\n")
# substitute and deparsing still see the calls, not the inlining.
cat(deparse(substitute(f(3))), "\n")
q <- function(x) substitute(x)
cat(deparse(q(f(3) + g1(2))), "\n")
# Recursion depth still errors through the real-call limit.
old <- options(expressions = 50)
r <- tryCatch(f(60), error = function(e) conditionMessage(e))
cat(r, "\n")
options(old)
# Rebinding the recursive name mid-flight is honored (guards).
f2 <- function(n) if (n < 2) n else f2(n - 1) + f2(n - 2)
f <- f2
cat(f(10), f2(10), "\n")
# Mixed types through nested inlines: doubles, integers, logicals.
m <- function(x) if (x > 0) m(x - 1L) else x
cat(m(7L), m(2.5), "\n")
k <- function(x) if (isTRUE(x)) 1L else k(FALSE)
cat(k(TRUE), "\n")
# A nested call result used several times and discarded.
u <- function(n) { v <- f(n); v + v + f(n - 1) }
cat(u(9), "\n")
cat("done\n")
