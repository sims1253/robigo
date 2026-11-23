# A function literal's compilerOptions reads (its make.functionContext) come
# where cmp.R's code generation meets the literal, before what the
# generation does after it: here the inlining's lookup that forces a delayed
# binding on a frame named package:base (the untaken branch: the compile
# forces it, not the run).
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)

e <- new.env()
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
attach(e, name = "package:base", warn.conflicts = FALSE)

f <- function(x) {
    for (i in 1:2) NULL
    g <- function() NULL
    if (x) nchar("abc")
    7
}
print(f(FALSE))
