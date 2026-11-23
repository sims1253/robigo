# cmpCall's findFunDef for a call the compiler does not inline: a lazy
# binding of the function's name is forced by the compile (its notes'
# check, made with the notes suppressed too), at the second call.
invisible(compiler::enableJIT(2))
e <- new.env()
delayedAssign("myfun", { cat("forced\n"); function(x) x }, assign.env = e)
attach(e, name = "lazy", warn.conflicts = FALSE)
f <- function(x) { if (x) myfun(x); 7 }
cat("call 1\n"); print(f(FALSE))
cat("call 2\n"); print(f(FALSE))
