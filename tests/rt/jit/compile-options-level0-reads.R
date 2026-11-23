# optimize 0, the JIT at level 2: cmp.R does not inline at level 0
# (getInlineInfo's optimize > 0), so a function literal's handler, and its
# function context's reads, never come; robigo-rt's compile declines and
# stops before the code's generation.
invisible(compiler::enableJIT(2))
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 0 }, o)
f <- function() { for (i in 1L) NULL; g <- function() 1; 7 }
print(f()); print(f())
