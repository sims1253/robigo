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
probe <- FALSE
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { print(probe); 2 }, o)
assign("+", .Primitive("<<-"), envir = env)
f <- function() {
    for (i in 1:2) NULL
    if (1L == 1L) g <- function() NULL
    if (`+`("probe", TRUE)) NULL
    3
}
r <- f()
assign("+", plus, envir = env)
print(r)
