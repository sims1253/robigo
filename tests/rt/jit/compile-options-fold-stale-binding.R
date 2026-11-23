# INTERIM(C1) (recorded, not ratcheted; #1029, cured by its PR, the compile's
# observation record): robigo-rt's compile retries after a fold or a read
# runs user code, and a retry re-reads what an earlier try read, where GNU
# R's cmp.R makes one pass; a try also compiles past an `if` whose test's
# fold is not known yet, so what follows it can come before what its branch
# does. GNU R's order here differs from robigo-rt's, as on main.
o <- compiler:::compilerOptions
env <- baseenv()
plus <- base::`+`
unlockBinding("+", env)
rm("optimize", envir = o)
n <- 0L
makeActiveBinding("optimize", function() {
    n <<- n + 1L
    if (n == 3L) {
        cat("rebind\n")
        assign("+", function(e1, e2) {
            if (identical(e1, 999L) && identical(e2, 999L)) {
                cat("fold\n")
                FALSE
            } else plus(e1, e2)
        }, envir = env)
    }
    2
}, o)
f <- function() {
    for (i in 1:2) NULL
    if (1L == 1L) g <- function() NULL
    if ((999L + 999L) == 1998L) return(42L)
    3L
}
print(f())
assign("+", plus, envir = env)
