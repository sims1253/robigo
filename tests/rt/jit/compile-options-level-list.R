# Recorded, not ratcheted: robigo-rt's relop does not compare a list yet
# (relop.c's coerceVectorList path, "robigo: comparing a list
# (coerceVectorList) is not implemented yet"), and getInlineInfo's tests of
# compilerOptions' optimize, made in R for a value that is not a plain
# number (compile/options.rs `level_in_r`), take that error as the tests'
# own: the compile is refused, silently. GNU R compares `list(2) > 0` as
# TRUE and compiles at level 2, forcing the lazy binding of `nchar`.
invisible(compiler::enableJIT(2))
o <- compiler:::compilerOptions
attach(NULL, name = "package:base", warn.conflicts = FALSE)
e <- as.environment(2)  # (the attached frame: attach() copies)
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
assign("optimize", list(2), envir = o)
f <- function(x) { if (x) nchar(x); 1 }
cat("list(2):", f(FALSE), f(FALSE), "\n")
assign("optimize", 2, envir = o)
