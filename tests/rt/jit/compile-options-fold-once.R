# A fold the compile evaluates (a classed constant in the code, its + method
# printing), before two function literals whose compilerOptions reads stop
# the compile: the fold is evaluated once, its value kept across the reads
# (cmp.R folds once).
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
`+.cls` <- function(e1, e2) { cat("fold\n"); TRUE }
f <- eval(substitute(function() {
    for (i in 1:2) NULL
    if (V + 1) NULL
    g <- function() NULL
    h <- function() NULL
    3
}, list(V = structure(1, class = "cls"))))
print(f())
