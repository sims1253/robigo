# A missing argument read by a byte-compiled closure is a getvarError
# (eval.c:getvar); read by the AST evaluator, eval() included, an
# evalError (eval.c:eval). GNU R's JIT compiles a closure at its second
# call, or at its first when it has a loop.
cls <- function(expr) paste(class(tryCatch(expr, error = identity)), collapse = "/")
f <- function(a) a
cls(f())
cls(f())
h <- function(a) eval(quote(a))
cls(h())
cls(h())
k <- function(a) { for (i in 1:2) NULL; a }
cls(k())
cls(eval(quote(expr = )))
# The AST evaluator also forces promises (eval.c:forcePromise) and runs
# on.exit code (context.c:R_run_onexits), in a compiled closure too (a
# closure with a loop is compiled at its first call).
f1 <- function(a) { for (i in 1) NULL; delayedAssign("x", a); x }
cls(f1())
f4 <- function(a) { for (i in 1) NULL; on.exit(a); 1 }
cls(f4())
f5 <- function(a, b = a) { for (i in 1) NULL; b }
cls(f5())
k2 <- function(a) { for (i in 1) NULL; a }
cls(k2())
