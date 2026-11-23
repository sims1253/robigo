# A function literal behind a fold (if (1 == 1)): its compilerOptions reads
# come before the later inlining's forced lookup, as cmp.R folds the
# condition and compiles the literal before it gets there.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
e <- new.env()
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
attach(e, name = "package:base", warn.conflicts = FALSE)
f <- function(x) {
    for (i in 1:2) NULL
    if (1 == 1) g <- function() NULL
    if (x) nchar("abc")
    7
}
print(f(FALSE))
