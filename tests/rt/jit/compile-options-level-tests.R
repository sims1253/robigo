# compilerOptions' optimize as getInlineInfo tests it (`optimize > 0`,
# `>= 3`, `>= 2`, in an `if`): a lazy binding of `nchar` in the global
# frames is forced by the compile where it looks the name up (a call it
# compiles, and findFunDef's check), at any level that compiles; NA, NaN,
# or a length other than one is the tests' error, and the compile fails
# at its first getInlineInfo, before anything is forced. A string
# compares as a string ("2" is level 2). Each closure's body is its own:
# GNU R's JIT cache reuses a body's code without compiling it again.
invisible(compiler::enableJIT(2))
o <- compiler:::compilerOptions
attach(NULL, name = "package:base", warn.conflicts = FALSE)
e <- as.environment(2)  # (the attached frame: attach() copies)
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
assign("optimize", NA, envir = o)
f <- function(x) { if (x) nchar(x); 1 }
cat("NA:", f(FALSE), f(FALSE), "\n")
assign("optimize", c(2, 2), envir = o)
f <- function(x) { if (x) nchar(x); 2 }
cat("c(2, 2):", f(FALSE), f(FALSE), "\n")
assign("optimize", integer(0), envir = o)
f <- function(x) { if (x) nchar(x); 3 }
cat("integer(0):", f(FALSE), f(FALSE), "\n")
assign("optimize", NaN, envir = o)
f <- function(x) { if (x) nchar(x); 4 }
cat("NaN:", f(FALSE), f(FALSE), "\n")
assign("optimize", 2i, envir = o)
f <- function(x) { if (x) nchar(x); 5 }
cat("2i:", f(FALSE), f(FALSE), "\n")
assign("optimize", TRUE, envir = o)
f <- function(x) { if (x) nchar(x); 6 }
cat("TRUE:", f(FALSE), f(FALSE), "\n")
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
assign("optimize", "2", envir = o)
f <- function(x) { if (x) nchar(x); 7 }
cat("\"2\":", f(FALSE), f(FALSE), "\n")
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
assign("optimize", 2.5, envir = o)
f <- function(x) { if (x) nchar(x); 8 }
cat("2.5:", f(FALSE), f(FALSE), "\n")
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
assign("optimize", -1, envir = o)
f <- function(x) { if (x) nchar(x); 9 }
cat("-1:", f(FALSE), f(FALSE), "\n")
assign("optimize", 2, envir = o)
