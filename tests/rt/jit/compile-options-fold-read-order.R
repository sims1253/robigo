# A function literal behind if (1 == 1), then a fold whose method prints:
# the literal's compilerOptions reads come before the fold's evaluation, as
# cmp.R's one pass meets them.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
`+.cls` <- function(e1, e2) { cat("fold\n"); TRUE }
f <- eval(substitute(function() {
    for (i in 1:2) NULL
    if (1 == 1) g <- function() NULL
    if (V + 1) NULL
    3
}, list(V = structure(1, class = "cls"))))
print(f())
