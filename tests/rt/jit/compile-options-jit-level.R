# compilerOptions' reads run as the compile's own R code: a read that turns
# the JIT level up is seen by the reads after it (tryCmpfun holds the level
# at 0 for the compile; no read resets it).
o <- compiler:::compilerOptions
rm("suppressAll", envir = o)
n <- 0L
makeActiveBinding("suppressAll", function() { n <<- n + 1L; if (n == 2L) compiler::enableJIT(1L); cat(compiler::enableJIT(-1L), "\n"); TRUE }, o)
f <- function() { for (i in 1:2) NULL; g <- function() 1; g() }
print(f())
