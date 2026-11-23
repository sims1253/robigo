# A fold whose method prints (a classed constant in the code) after an
# if (1 == 1) branch whose inlining forces a delayed binding: the branch is
# compiled, and the binding forced, before the later fold is evaluated, as
# cmp.R's one pass meets them (robigo-rt's compile evaluates only the first
# of such folds a guess met, then compiles again).
`+.cls` <- function(e1, e2) { cat("fold\n"); TRUE }
e <- new.env()
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
attach(e, name = "package:base", warn.conflicts = FALSE)
f <- eval(substitute(function(x) {
    for (i in 1:2) NULL
    if (1 == 1) { if (x) nchar("abc") }
    if (V + 1) NULL
    if (x) nchar("def")
    7
}, list(V = structure(1, class = "cls"))))
print(f(FALSE))
