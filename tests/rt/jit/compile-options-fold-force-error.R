# A lookup the compile forces that fails (a delayed binding whose promise
# errors): the compile fails there, as cmp.R's does, before the later
# function literal, whose reads therefore never come; the closure runs
# uncompiled.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
e <- new.env()
delayedAssign("nchar", { cat("forced\n"); stop("no nchar") }, assign.env = e)
attach(e, name = "package:base", warn.conflicts = FALSE)
f <- function(x) {
    for (i in 1:2) NULL
    if (1 == 1) { if (x) nchar("abc") }
    g <- function() NULL
    7
}
print(f(FALSE))
