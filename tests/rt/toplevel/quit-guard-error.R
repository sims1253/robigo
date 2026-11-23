# An error in .Last inside quit(): it lands on the top level the reset
# made, R halts (status 1), and no frame the reset abandoned runs again
# (f()'s on.exit).
.Last <- function() stop("in .Last")
f <- function() { on.exit(cat("f on.exit\n")); quit("no", 3) }
f()
