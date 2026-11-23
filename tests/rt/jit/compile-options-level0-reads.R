# optimize 0, the JIT at level 2: cmp.R does not inline at level 0
# (getInlineInfo's optimize > 0), so a function literal is a call of
# `function`, compiled as a call: its handler, and its function context's
# reads, never come. At TRUE (1) it inlines, and the literal reads.
invisible(compiler::enableJIT(2))
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 0 }, o)
f <- function() { for (i in 1L) NULL; g <- function() 1; 7 }
print(f()); print(f())
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read TRUE\n"); TRUE }, o)
# (a body of its own: GNU R's JIT cache would reuse the first one's code)
f <- function() { for (i in 1L) NULL; g <- function() 2; 8 }
print(f()); print(f())
