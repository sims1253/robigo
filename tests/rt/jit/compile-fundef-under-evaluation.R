# findFunDef of a name whose lazy binding is being evaluated: get()'s
# "promise already under evaluation" fails the compile (tryCmpfun's
# handler, silent with the notes suppressed), and the closure runs
# uncompiled; the promise's own value comes after.
invisible(compiler::enableJIT(2))
h <- function() { if (FALSE) g(); 1 }
delayedAssign("g", { cat("forcing g\n"); h(); h(); h(); function() 2 })
print(g())
print(h())
