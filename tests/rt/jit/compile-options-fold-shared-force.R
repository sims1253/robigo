# As compile-options-fold-force-order.R, with one call object (P) in both
# conditions: the folds it shares are one request, which the later
# condition's names; still the earlier branch is compiled, and its binding
# forced, before the later fold is evaluated.
`+.cls` <- function(e1, e2) { cat("fold\n"); TRUE }
e <- new.env()
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
attach(e, name = "package:base", warn.conflicts = FALSE)
f <- eval(substitute(function(x) {
    for (i in 1:2) NULL
    if (P) { if (x) nchar("abc") }
    if (P && (V + 1)) NULL
    if (x) nchar("def")
    7
}, list(P = quote(1 == 1), V = structure(1, class = "cls"))))
print(f(FALSE))
