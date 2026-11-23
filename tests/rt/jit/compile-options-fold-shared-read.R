# As compile-options-fold-read-order.R, with one call object (P) in both
# conditions: the earlier branch's function literal is read before the
# later fold is evaluated.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
`+.cls` <- function(e1, e2) { cat("fold\n"); TRUE }
f <- eval(substitute(function() {
    for (i in 1:2) NULL
    if (P) g <- function() NULL
    if (P && (V + 1)) NULL
    3
}, list(P = quote(1 == 1), V = structure(1, class = "cls"))))
print(f())
