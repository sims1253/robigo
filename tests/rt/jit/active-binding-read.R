# Byte code reads an active binding twice per variable read: bcEval's
# inlined GETVAR looks the variable up (the binding's function runs) to
# see whether it is a promise to force in place, then getvar() reads it
# again (eval.c DO_GETVAR, INLINE_GETVAR). The AST reads it once.
n <- 0L
reads <- function(f) { n0 <- n; f(); n - n0 }
makeActiveBinding("ab", function() { n <<- n + 1L; 1 }, environment())
# a top-level closure, by its calls (the JIT compiles it at the second)
g <- function() ab
for (k in 1:4) cat("call", k, "reads", reads(g), "\n")
# compiled by cmpfun, a loop of three reads
h <- compiler::cmpfun(function() ab)
cat("cmpfun", reads(h), "\n")
l <- compiler::cmpfun(function() { x <- 0; for (i in 1:3) x <- x + ab; x })
cat("loop", reads(l), "\n")
# an active binding of an enclosing environment, and of the function's own frame
e <- new.env()
makeActiveBinding("ac", function() { n <<- n + 1L; 2 }, e)
k <- compiler::cmpfun(eval(quote(function() ac), e))
cat("enclosing", reads(k), "\n")
f <- compiler::cmpfun(function() {
  makeActiveBinding("q", function() { n <<- n + 1L; 3 }, environment())
  q
})
cat("frame", reads(f), "\n")
# the AST profile: once
u <- function() ab
compiler::enableJIT(0)
cat("ast", reads(u), reads(u), reads(u), "\n")
# an error in the binding's function: from the first read
makeActiveBinding("bad", function() { n <<- n + 1L; stop("no value") }, environment())
b <- compiler::cmpfun(function() bad)
n0 <- n
print(tryCatch(b(), error = conditionMessage))
cat("error reads", n - n0, "\n")
