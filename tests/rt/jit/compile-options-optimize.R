# compilerOptions' optimize at levels 0 and 1 (with the JIT at level 2:
# setCompilerOptions refuses optimize < 2 with the JIT above 2): cmp.R
# inlines less, and the results are the evaluator's; robigo-rt's compile
# declines below level 2, and the closure runs uncompiled (the same output).
# (No top-level loop: one would be compiled whole, its closures with it,
# before an option set inside it took effect.)
invisible(compiler::enableJIT(2))
compiler::setCompilerOptions(optimize = 0)
f0 <- function(x) { s <- 0; for (i in seq_len(x)) s <- s + sqrt(i) * 2; c(s, length(x), is.null(x)) }
print(f0(4))
g0 <- function(v) { for (i in 1:2) NULL; v[[2]] + max(v) }
print(g0(c(3, 5, 7)))
compiler::setCompilerOptions(optimize = 1)
f1 <- function(x) { s <- 0; for (i in seq_len(x)) s <- s + sqrt(i) * 2; c(s, length(x), is.null(x)) }
print(f1(4))
g1 <- function(v) { for (i in 1:2) NULL; v[[2]] + max(v) }
print(g1(c(3, 5, 7)))
