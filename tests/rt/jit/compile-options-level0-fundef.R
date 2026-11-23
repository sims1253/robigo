# optimize 0: nothing inlines, so `if (x) nchar(x)` is a call of `if`,
# whose arguments' code is compiled (cmpCallArgs), and `nchar(x)` is a
# call cmpCall checks with findFunDef: the lazy binding of `nchar` is
# forced at the second call's compile, as at level 2.
invisible(compiler::enableJIT(2))
invisible(compiler::setCompilerOptions(optimize = 0L))
e <- new.env()
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
attach(e, name = "package:base", warn.conflicts = FALSE)
f <- function(x) { if (x) nchar(x); 7 }
cat("call 1\n"); print(f(FALSE))
cat("call 2\n"); print(f(FALSE))
invisible(compiler::setCompilerOptions(optimize = 2L))
