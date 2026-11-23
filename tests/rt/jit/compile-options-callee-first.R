# A call whose function is computed (a function literal called at once):
# the function's code before the arguments' promise codes, as cmp.R's
# cmpCallExprFun, so a lookup it forces comes between the right reads.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
e <- new.env()
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
attach(e, name = "package:base", warn.conflicts = FALSE)
h <- function(x) { for (i in 1:2) NULL; (function(a) { if (x) nchar(x); 7 })(function() NULL) }
print(h(FALSE))
