# compilerOptions are read in every compile, as cmp.R reads them: at the
# compile's entry (make.toplevelContext) and again for each function
# context (make.functionContext: the closure's body, and each `function`
# literal its code holds). An active binding on `optimize` counts them.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
n <- 0L
makeActiveBinding("optimize", function() { n <<- n + 1L; 2 }, o)
f <- function() { for (i in 1:2) NULL; g <- function(x) x + 1; g(1) }
print(f())
cat("reads for a closure with one function literal:", n, "\n")
n <- 0L
h <- function() { for (i in 1:2) NULL; 0 }
print(h())
cat("reads for a closure with none:", n, "\n")
