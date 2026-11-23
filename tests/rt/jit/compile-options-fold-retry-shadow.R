# (g3) A fold whose method defines a global `+` after `1 + 2` was folded: GNU R
# keeps the constant 3; robigo-rt's retry re-reads isBaseVar("+") and compiles
# the prefix as a call to the global `+` (99), as on main 1972bb945.
# INTERIM(C1) (recorded, not ratcheted; #1029, cured by its PR, the compile's
# observation record): robigo-rt's compile retries after a fold or a read
# runs user code, and a retry re-reads what an earlier try read, where GNU
# R's cmp.R makes one pass; a try also compiles past an `if` whose test's
# fold is not known yet, so what follows it can come before what its branch
# does. GNU R's order here differs from robigo-rt's, as on main.
`-.cls` <- function(e1, e2) {
    cat("shadow +\n")
    assign("+", function(e1, e2) { cat("global +\n"); 99 }, envir = globalenv())
    0
}
f <- eval(substitute(function() {
    for (i in 1:2) NULL
    a <- 1 + 2
    b <- V - 1
    a
}, list(V = structure(1, class = "cls"))))
print(f())
print(f())
