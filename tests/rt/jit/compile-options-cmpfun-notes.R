# An explicit compiler::cmpfun runs cmp.R's own R code, notes included:
# with suppressAll FALSE it prints them as GNU R does (the JIT's compile,
# robigo-rt's own, does not have them yet: compile-options-suppressall.R).
compiler::setCompilerOptions(suppressAll = FALSE)
f <- function() { undefinedVariableX + 1; undefinedFunctionY(2) }
g <- compiler::cmpfun(f)
cat(typeof(.Internal(bodyCode(g))), "\n")
