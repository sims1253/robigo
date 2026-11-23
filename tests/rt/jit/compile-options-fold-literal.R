# INTERIM(C1) (recorded, not ratcheted; #1029, cured by its PR, the compile's
# observation record): robigo-rt's compile retries after a fold or a read
# runs user code, and a retry re-reads what an earlier try read, where GNU
# R's cmp.R makes one pass; a try also compiles past an `if` whose test's
# fold is not known yet, so what follows it can come before what its branch
# does. GNU R's order here differs from robigo-rt's, as on main.
# A function literal behind a fold (if (1 == 1)): its compilerOptions reads
# come before the later inlining's forced lookup, as cmp.R folds the
# condition and compiles the literal before it gets there.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
e <- new.env()
delayedAssign("nchar", { cat("forced\n"); base::nchar }, assign.env = e)
attach(e, name = "package:base", warn.conflicts = FALSE)
f <- function(x) {
    for (i in 1:2) NULL
    if (1 == 1) g <- function() NULL
    if (x) nchar("abc")
    7
}
print(f(FALSE))
