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
