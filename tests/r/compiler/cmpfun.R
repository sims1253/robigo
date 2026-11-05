# compiler::cmpfun() and compile() run GNU R's own compiler (cmp.R) on the
# internals bcVersion, mkCode, putconst, getconst, growconst and bcClose.
# The compiled closure has the same formals, body() and environment, prints
# with <bytecode>, and differs from the source only for identical(...,
# ignore.bytecode = FALSE).
addr <- function(x) {
  tc <- textConnection("out", "w", local = TRUE)
  sink(tc); print(x); sink(); close(tc)
  sub("0x[0-9a-f]+", "ADDR", out)
}
f <- function(x, y = 2) x + y
g <- compiler::cmpfun(f)
typeof(g)
g(1)
g(1, 10)
body(g)
formals(g)
identical(environment(g), environment(f))
identical(f, g)
identical(f, g, ignore.bytecode = FALSE)
identical(g, g, ignore.bytecode = FALSE)
addr(g)
deparse(g)

# free variables, recursion, loops and closures created by compiled code
k <- compiler::cmpfun(function(n) if (n <= 1) 1 else n * k(n - 1))
k(10)
make <- compiler::cmpfun(function(a) function(b) a * b)
make(3)(4)
s <- compiler::cmpfun(function(v) { t <- 0; for (e in v) t <- t + e; t })
s(1:100)
m <- compiler::cmpfun(function(...) list(...))
m(1, b = "two")

# attributes of the function are kept
h <- structure(function() "h", class = c("myfun", "function"), note = "kept")
hc <- compiler::cmpfun(h)
class(hc)
attr(hc, "note")
hc()

# primitives are returned as they are; other values are an error
identical(compiler::cmpfun(sum), sum)
tryCatch(compiler::cmpfun(1), error = conditionMessage)

# compile() returns a code object that eval() runs
code <- compiler::compile(quote(1 + 2 * 3))
typeof(code)
eval(code)
x <- 5
eval(compiler::compile(quote(x * 2)))
