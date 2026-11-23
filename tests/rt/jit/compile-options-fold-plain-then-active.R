# compilerOptions plain until a binding the compile forces makes one active:
# the function literal after the force is read then (a guess past a fold
# counts no read as made).
# (the compiler loaded before the compile, as GNU R's JIT has it from its
# start; the M1 boot loads it on demand, INTERIM(L5))
invisible(compiler:::compilerOptions)
e <- new.env()
delayedAssign("nchar", {
    cat("forced\n")
    o <- compiler:::compilerOptions
    rm("optimize", envir = o)
    makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
    base::nchar
}, assign.env = e)
attach(e, name = "package:base", warn.conflicts = FALSE)
f <- function(x) {
    for (i in 1:2) NULL
    if (1 == 1) { if (x) nchar("abc") }
    g <- function() NULL
    7
}
print(f(FALSE))
