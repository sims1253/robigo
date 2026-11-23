# compilerOptions' reads in the order of cmp.R's compile: the entry's
# (make.toplevelContext), then the function context's after its funEnv
# (make.functionContext), and only then the code's generation, which here
# forces a delayed binding the call's inlining looks up (an attached
# frame's `nchar`). An active binding on `optimize` shows each read.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
e <- new.env()
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
attach(e, name = "shadow", warn.conflicts = FALSE)
f <- function(x) { for (i in 1:2) NULL; nchar(x) }
print(f("abc"))
