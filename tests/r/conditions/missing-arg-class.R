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
# A compiled caller's argument promise is bytecode: forcing it in the
# callee reads with getvar, whether or not the callee is compiled; an
# interpreted caller's promise is read by the AST evaluator.
fa <- function(arg) { for (i in 1) NULL; is.factor(arg) }
ga <- function(x) { for (i in 1) NULL; fa(x) }
cls(ga())
gb <- function(x) fa(x)
cls(gb())
fc <- function(arg) is.factor(arg)
gc <- function(x) { for (i in 1) NULL; fc(x) }
cls(gc())
fd <- function(arg) is.factor(arg)
gd <- function(x) fd(x)
cls(gd())
