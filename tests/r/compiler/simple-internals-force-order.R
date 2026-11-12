# GNU R's compiler gets each wrapper's definition (cmp.R:findFunDef) as it
# reaches the call, so lazy bindings are forced in the order the code
# calls them. (robigo forces the inlined wrappers in table order.)
orig <- baseenv()$nchar
orig2 <- baseenv()$typeof
unlockBinding("nchar", baseenv()); unlockBinding("typeof", baseenv())
delayedAssign("nchar", { cat("nchar forced\n"); orig }, assign.env = baseenv())
delayedAssign("typeof", { cat("typeof forced\n"); orig2 }, assign.env = baseenv())
f <- function(a) if (a) { typeof(1); nchar("abc") } else 0L
f(FALSE); f(FALSE)
assign("nchar", orig, baseenv()); assign("typeof", orig2, baseenv())
lockBinding("nchar", baseenv()); lockBinding("typeof", baseenv())
