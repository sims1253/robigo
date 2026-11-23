# INTERIM(C1) (recorded, not ratcheted; #1029, cured by its PR, the compile's
# observation record): robigo-rt's compile retries after a fold or a read
# runs user code, and a retry re-reads what an earlier try read, where GNU
# R's cmp.R makes one pass; a try also compiles past an `if` whose test's
# fold is not known yet, so what follows it can come before what its branch
# does. GNU R's order here differs from robigo-rt's, as on main.
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
