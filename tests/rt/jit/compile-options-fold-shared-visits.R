# INTERIM(C1) (recorded, not ratcheted; #1029, cured by its PR, the compile's
# observation record): robigo-rt's compile retries after a fold or a read
# runs user code, and a retry re-reads what an earlier try read, where GNU
# R's cmp.R makes one pass; a try also compiles past an `if` whose test's
# fold is not known yet, so what follows it can come before what its branch
# does. GNU R's order here differs from robigo-rt's, as on main.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
n <- 0L
`+.cls` <- function(e1, e2) { n <<- n + 1L; cat("fold\n"); n == 1L }
p <- substitute(V + 1, list(V = structure(1, class = "cls")))
f <- eval(substitute(function() {
    for (i in 1:2) NULL
    if (P) NULL
    g <- function() NULL
    if (P) return(42L)
    3
}, list(P = p)))
print(f())
