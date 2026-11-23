# INTERIM(C1) (recorded, not ratcheted; #1029, cured by its PR, the compile's
# observation record): robigo-rt's compile retries after a fold or a read
# runs user code, and a retry re-reads what an earlier try read, where GNU
# R's cmp.R makes one pass; a try also compiles past an `if` whose test's
# fold is not known yet, so what follows it can come before what its branch
# does. GNU R's order here differs from robigo-rt's, as on main.
# A fold whose function is a closure bound in base in place of the
# primitive (it prints for 999L + 999L): its evaluation can be seen, so it
# comes after the earlier branch's function literal's reads, as in cmp.R's
# one pass.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
plus <- base::`+`
env <- baseenv()
unlockBinding("+", env)
assign("+", function(e1, e2) { if (identical(e1, 999L) && identical(e2, 999L)) { cat("fold\n"); FALSE } else plus(e1, e2) }, envir = env)
f <- function() {
    for (i in 1:2) NULL
    if (1 == 1) g <- function() NULL
    if (999L + 999L) NULL
    3
}
print(f())
assign("+", plus, envir = env)
